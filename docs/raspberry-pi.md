# Faire tourner le projet sur un Raspberry Pi 4

Le dépôt vise une machine allumée en permanence, silencieuse et qui ne coûte rien à
laisser tourner. Un Pi 4 fait exactement ça, et le projet s'y prête mieux qu'il n'y
paraît — non par chance, mais parce que des choix pris pour d'autres raisons se
révèlent être ceux qu'il fallait.

## Pourquoi ça marche

**L'arbre de dépendances est entièrement en Rust.** Il l'est devenu en retirant le
codec `zstd` de `parquet` : c'était le seul morceau de C du dépôt, il tirait `cc` et
`pkg-config`, et nous ne nous en servions jamais — rien ne règle la compression à
l'écriture, donc parquet écrit en clair. Les autres codecs restent, tous en Rust pur,
parce qu'ils servent à **lire** ce que d'autres outils ont écrit.

**`chrono-tz` embarque la base IANA en données statiques.** Le choix avait été fait
pour que `gex-core` ne puisse pas lire `/usr/share` ; il fait qu'un Pi fraîchement
flashé, sans tzdata configuré, calcule quand même la bascule de séance à 17 h
New York.

**L'empreinte est dérisoire.** Mesurée sur la machine de développement :

| | mémoire | pic |
|---|---|---|
| `gex-collector` | 31,4 Mo | 32,5 Mo |
| `gex-web` | 25,7 Mo | 27,5 Mo |

Côté disque, `snapshots/` pèse un demi-mégaoctet pour trois fichiers. Avec les
archives et sept jours de rétention on reste dans les dizaines de mégaoctets — une
carte SD ne s'en aperçoit pas.

**Le code compile pour `aarch64-unknown-linux-gnu`** sans une seule modification, ce
qui a été vérifié sur l'ensemble du workspace, tests et exemples compris.

## Le matériel

**Prends le modèle 8 Go, ou au minimum 4 Go.** Les chiffres ci-dessus ne comptent que
la partie Rust. IB Gateway est une machine virtuelle Java : compte 300 à 700 Mo de
plus. C'est Gateway qui dimensionne la machine, pas nous.

Une carte SD suffit, mais un SSD USB vaut mieux — pas pour la place, pour l'usure :
le collecteur réécrit `courant.parquet` toutes les quinze secondes.

## IB Gateway sur ARM64

IBKR publie désormais un installeur `linux-arm` **officiel**, à partir de la version
`10.37.1l` sur le canal *latest* et `10.39.1e` sur le canal *stable*. Ce n'est plus le
bricolage d'autrefois — poser un JRE ARM64 et lancer les jars à la main — qui n'est
donc plus nécessaire.

TWS, lui, n'a pas de version ARM64. Cela ne nous concerne pas : le collecteur parle à
l'API, jamais à l'interface graphique.

**Gateway reste une application graphique.** Sur une machine sans écran il lui faut
donc un serveur X virtuel — `Xvfb` — et, pour que la connexion se refasse seule après
le redémarrage quotidien d'IB, un automate : `IBC`. L'image `ib-gateway-docker` monte
exactement cet assemblage et sert de référence utile.

> À vérifier sur place. Tout ce qui précède sur Gateway vient de sa documentation et
> de projets tiers, pas d'un essai fait ici : il n'y avait pas de Pi sous la main. Le
> reste de cette page — compilation, empreinte, services — a été mesuré.

## Compiler

> Cette section est devenue facultative : `release.yml` construit les binaires
> ARM64 et les publie, `scripts/gex-maj.sh` les installe. Compiler sur le Pi ne
> sert plus qu'à essayer une modification avant de la publier.

**Sur le Pi directement, c'est le plus simple.** Avec 8 Go, la phase d'optimisation
finale passe sans difficulté. Compte trente à soixante minutes la première fois ;
ensuite les compilations sont incrémentales et brèves.

```sh
sudo apt update && sudo apt install -y git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level.git
cd Option-Gamma-Exposure--zero-gamma-level
cargo build --release --manifest-path options-rs/Cargo.toml
```

La compilation croisée depuis Windows est possible mais demande davantage
d'installation : une distribution WSL, puis `gcc-aarch64-linux-gnu` pour l'édition de
liens. La cible seule ne suffit pas — `cargo check` se passe d'éditeur de liens,
`cargo build` non. Pour deux binaires compilés une fois de temps en temps, compiler
sur le Pi épargne tout ce montage.

## Les deux services

Les deux sont indépendants, et c'est voulu : le lecteur ne va jamais chercher de
données, donc l'écran continue de servir pendant que le collecteur encaisse le
redémarrage quotidien d'IB.

```ini
# /etc/systemd/system/gex-collector.service
[Unit]
Description=Collecteur GEX — balaie la chaîne d'options via IB Gateway
After=network-online.target ibgateway.service
Wants=network-online.target

[Service]
Type=simple
User=gex
WorkingDirectory=/opt/gex
ExecStart=/opt/gex/bin/gex-collector NQ --retention 7
# IB coupe la session une fois par jour : le collecteur doit y survivre seul.
Restart=always
RestartSec=30
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

```ini
# /etc/systemd/system/gex-web.service
[Unit]
Description=Écran de séance GEX
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=gex
WorkingDirectory=/opt/gex
ExecStart=/opt/gex/bin/gex-web NQ --ecoute 0.0.0.0
Restart=always
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/gex/snapshots

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now gex-collector gex-web
journalctl -u gex-collector -f
```

## Joindre l'écran depuis une autre machine

`gex-web` écoute sur le loopback par défaut, et **ce défaut est un choix** : ces
relevés sont à toi. Sur une machine sans écran il n'a plus de sens, puisque
l'interface se consulte forcément d'ailleurs — d'où `--ecoute 0.0.0.0`, qui doit
rester écrit à la main. Un test le verrouille : personne ne basculera ce défaut par
distraction.

Ce que ça expose reste une page en lecture seule, mais elle est alors visible de tout
ton réseau local. Si ce réseau n'est pas de confiance, garde le loopback et passe par
un tunnel SSH :

```sh
ssh -L 8787:127.0.0.1:8787 gex@raspberrypi
```

L'écran est ensuite sur `http://127.0.0.1:8787` depuis ton poste, sans rien ouvrir.

## Y accéder depuis n'importe où

La question n'est pas de savoir si ton écran de gamma est un secret. C'est de
savoir **ce qu'il y a d'autre sur cette machine** : le Pi tient une session IB
Gateway authentifiée sur ton compte réel, et l'API de cette session sait passer
des ordres. Ce dépôt ne le fait jamais — mais c'est une propriété de ce code, pas
de la machine. Rediriger un port vers un service sans authentification, devant une
machine qui porte cette session, est le seul montage vraiment à éviter.

Il n'y en a d'ailleurs pas besoin.

### Tailscale : le plus simple et le plus sûr à la fois

Les deux vont rarement ensemble. Tailscale monte un réseau privé entre tes
appareils — le Pi, ton portable, ton téléphone. Aucune redirection de port, aucun
DNS dynamique à entretenir, aucun certificat. Gratuit pour un usage personnel,
paquet ARM64 disponible.

```sh
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
tailscale ip -4        # l'adresse du Pi sur ton reseau prive, en 100.x.y.z
```

Fais ensuite écouter l'écran **sur cette adresse-là** plutôt que sur `0.0.0.0`,
dans `gex-web.service` :

```ini
ExecStart=/opt/gex/bin/gex-web NQ --ecoute 100.x.y.z
```

L'écran n'est alors même plus visible depuis ton réseau local : seuls tes propres
appareils l'atteignent, où qu'ils soient. Avec MagicDNS, l'adresse devient
`http://raspberrypi:8787` depuis n'importe lequel d'entre eux.

### Cloudflare Tunnel, si tu veux une vraie URL publique

Pour ouvrir l'écran depuis un appareil sur lequel tu ne peux rien installer,
`cloudflared` monte un tunnel **sortant** — donc toujours aucun port ouvert — et
rend une adresse en HTTPS. Mets Cloudflare Access devant : sans lui, l'adresse est
publique et l'écran n'a aucune authentification à lui. C'est nettement plus de
montage que Tailscale, pour un besoin que tu n'as peut-être pas.

## Mettre à jour sans recompiler

Deux workflows vivent dans `.github/workflows`.

**`ci.yml`** rejoue les tests et clippy à chaque poussée, sur x86_64 **et** sur
aarch64. Le second est celui qui compte ici : il exécute vraiment la suite sur
l'architecture du Pi, ce que la machine de développement ne sait pas faire.

**`release.yml`** construit les binaires ARM64 et les attache à une version. Le
runner aarch64 de GitHub est gratuit sur un dépôt public et compile nativement,
donc le Pi n'a plus jamais à compiler.

Publier une version :

```sh
git tag v0.2.0 && git push origin v0.2.0
```

Mettre le Pi à jour, depuis le Pi :

```sh
./scripts/gex-maj.sh
```

Le script télécharge la dernière archive, **vérifie sa somme de contrôle avant de
toucher aux binaires en service**, remplace les trois et redémarre les deux
services. Le Pi va chercher : rien ne pousse vers lui, aucun port entrant, aucun
runner sur la machine qui tient la session Gateway. C'est ce qui permet de garder
le dépôt public sans exposer la collecte.

Lance-le **hors séance** : l'arrêt du collecteur coûte le balayage en cours, une
quinzaine de minutes avant que la chaîne soit de nouveau complète.

## Ce qui reste à faire sur place

1. Installer Gateway ARM64 et vérifier qu'il se connecte, avec `Xvfb` et `IBC`.
2. Autoriser l'API dans Gateway, et y déclarer l'adresse du Pi si Gateway tourne
   ailleurs (`--adresse` côté collecteur pointe alors vers cette machine).
3. Poser Tailscale, relever l'adresse `100.x.y.z` et l'écrire dans
   `gex-web.service`.
4. Confirmer qu'une séance entière tient sans redémarrage, et relever la mémoire
   réellement prise par Gateway.

Les tests sur l'architecture, eux, n'ont plus à être refaits à la main : la CI les
rejoue en aarch64 à chaque poussée.
