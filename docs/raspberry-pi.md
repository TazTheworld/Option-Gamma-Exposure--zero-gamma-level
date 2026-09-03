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

**L'empreinte est dérisoire**, et ce n'est pas nous qui dimensionnons la machine.
Mesurée sur le Pi, en séance, tout en marche :

| | mémoire |
|---|---|
| IB Gateway (JVM) | **744 Mo** |
| `Xvfb` | 27 Mo |
| `gex-collector` | 22 Mo |
| `gex-web` | 4 Mo |

Gateway pèse vingt-cinq fois les deux binaires réunis. C'est lui, et lui seul,
qui impose de prendre 8 Go plutôt que 2.

Côté disque, `snapshots/` pèse un demi-mégaoctet pour trois fichiers. Avec la
rétention par défaut de trente jours et sans archives à la minute — le défaut
aussi — on reste dans les dizaines de mégaoctets ; une carte SD ne s'en aperçoit
pas. Ce sont les archives, si on les allume, qui coûtent : compter dix gigaoctets
pour trente jours.

Côté réseau, l'écran est le seul poste qui compte : il redemande les trois séries
toutes les quinze secondes. Elles pesaient **1 028 Ko par passage** — 4 Mo par
minute et par onglet ouvert, portés par le Wi-Fi du Pi. Depuis que `gex-web`
comprime lui-même, c'est **169 Ko**, et une requête en réseau local est passée de
0,36 à 0,09 s. Cloudflare comprimait déjà pour les navigateurs venus du tunnel,
ce qui masquait le coût vu de l'extérieur mais ne l'enlevait ni du trajet
Pi → Cloudflare, ni des accès locaux.

**Le code compile pour `aarch64-unknown-linux-gnu`** sans une seule modification, ce
qui a été vérifié sur l'ensemble du workspace, tests et exemples compris.

## Le matériel

**Prends le modèle 8 Go, ou au minimum 4 Go.** Les chiffres ci-dessus ne comptent que
la partie Rust. IB Gateway est une machine virtuelle Java : compte 300 à 700 Mo de
plus. C'est Gateway qui dimensionne la machine, pas nous.

Une carte SD suffit, mais un SSD USB vaut mieux — pas pour la place, pour l'usure :
le collecteur réécrit `courant.parquet` toutes les quinze secondes.

## La distribution

**Raspberry Pi OS Lite, 64 bits.** La version courante repose sur Debian 13
« Trixie ».

Le **64 bits** n'est pas négociable : l'installeur ARM d'IB Gateway est en
aarch64, et les binaires publiés par la CI aussi. Une image 32 bits est une
impasse dont on ne sort qu'en réinstallant. **Lite** veut dire sans bureau, ce
qu'on cherche puisque la machine n'aura pas d'écran — Xvfb viendra ensuite pour
Gateway seul, plutôt qu'un environnement graphique complet qui tournerait pour
rien.

### Pourquoi ne pas chercher plus léger

Le réflexe est bon mais il ne s'applique pas ici. Raspberry Pi OS Lite occupe
environ 200 Mo au repos, DietPi descend sous 40 Mo — et **Gateway en prendra 700
à lui seul**. Économiser cent cinquante mégaoctets sur le système quand la JVM en
consomme cinq fois plus ne change rien de mesurable, et DietPi ajoute une couche
de configuration qui, elle, se paie en temps.

Le critère utile n'est donc pas l'empreinte mais le support matériel et la
stabilité, et c'est Raspberry Pi OS qui gagne sur les deux : il est fait pour
cette carte.

### Et Ubuntu Server ?

Il ferait très bien l'affaire en 24.04 LTS ARM64, et son support long est un vrai
argument pour une machine qu'on veut oublier pendant des années. Deux réserves :
il charge sur un Pi des modules noyau dont il n'a pas l'usage, et le support de
la carte reste meilleur du côté de Raspberry Pi OS.

### L'installation

Passe par **Raspberry Pi Imager**, qui règle tout avant le premier démarrage —
utilisateur, mot de passe, SSH activé, Wi-Fi. Sans lui il faut un clavier et un
écran pour la première connexion, ce qui est absurde pour une machine destinée à
n'en avoir jamais.

Nos binaires ne demandent rien d'autre que la glibc du système. Gateway, lui, est
une application graphique et réclame de quoi en faire tourner une sans écran :

```sh
sudo apt update
sudo apt install -y xvfb libxtst6 libxrender1 libxi6 curl
```

L'installeur officiel de Gateway apporte normalement son propre JRE ; s'il ne le
fait pas, `openjdk-17-jre` est dans les dépôts ARM64 de Debian. Une chaîne Rust
n'est nécessaire que si tu veux compiler sur place, ce dont la CI te dispense.

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

Tout ce qui précède a maintenant été fait sur une machine réelle — un Pi 4 de
8 Go sous Raspberry Pi OS Lite 64 bits, Gateway 10.45 et IBC 3.24.2. Les pièges
rencontrés, qui ne sont dans aucune documentation, sont consignés dans
`AGENTS.md`.

## Fermer le port de l'API

Gateway écoute sur `*:4001`, et cette session sait passer des ordres. Deux
réglages de `jts.ini` la bornent — `ApiOnly=true`, et `TrustedIPs=127.0.0.1` qui
fait refuser toute session venue d'ailleurs. Le port reste pourtant joignable
depuis le réseau local, donc sondable.

**Il n'existe pas de réglage IBC pour le fermer.** Son `BindAddress` gouverne le
serveur de commandes d'IBC, pas l'API — le nom trompe, et l'essai est sans effet
sur `ss -ltn`. Le seul moyen est un pare-feu sur la machine :

```sh
sudo apt install -y nftables
sudo tee /etc/nftables.conf >/dev/null <<'EOF'
#!/usr/sbin/nft -f
flush ruleset
table inet filtre {
    chain entree {
        type filter hook input priority 0; policy accept;
        iif "lo" accept
        tcp dport 4001 drop
    }
}
EOF
sudo systemctl enable --now nftables
```

La politique reste `accept` et la règle est unique : ce fichier ne ferme rien
d'autre, et une erreur ici ne doit pas couper SSH. `iif "lo" accept` laisse
passer le collecteur, qui tourne sur la même carte.

**Le vérifier depuis une autre machine, jamais depuis le Pi.** Une connexion du
Pi vers sa propre adresse passe par `lo` et tombe sur la règle d'acceptation :
le test réussit et ne prouve rien.

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
Documentation=https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level
After=network-online.target ibgateway.service
Wants=network-online.target

[Service]
Type=simple
User=gex
Group=gex
WorkingDirectory=/opt/gex
# --adresse est indispensable : le défaut du binaire est 7496, le port de TWS.
# Gateway écoute sur 4001, et le pare-feu n'y laisse passer que la boucle locale.
ExecStart=/opt/gex/bin/gex-collector NQ --adresse 127.0.0.1:4001

# Les alertes vivent entièrement dans ce fichier, absent par défaut. Y écrire
# GEX_ALERTE_COMMANDE=/opt/gex/bin/alerte-discord.sh et DISCORD_WEBHOOK=... les
# allume au redémarrage suivant. Tant qu'il n'existe pas, les bascules ne sont
# qu'écrites au journal — plutôt qu'un envoi qui échouerait à chaque passage.
EnvironmentFile=-/etc/gex.env

# IB coupe la session une fois par jour : le collecteur doit y survivre seul.
Restart=always
RestartSec=30

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/gex
# AF_UNIX et AF_NETLINK ne sont pas décoratifs : sans eux getaddrinfo échoue.
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX AF_NETLINK

[Install]
WantedBy=multi-user.target
```

```ini
# /etc/systemd/system/gex-web.service
[Unit]
Description=Écran de séance GEX
Documentation=https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=gex
Group=gex
WorkingDirectory=/opt/gex
# --ecoute 0.0.0.0 est écrit à la main, et un test du dépôt verrouille le défaut
# sur la boucle locale : sur une machine sans écran l'interface se consulte
# forcément d'ailleurs. Ce qui est exposé reste une page en lecture seule.
ExecStart=/opt/gex/bin/gex-web NQ --ecoute 0.0.0.0 --port 8787

Restart=always
RestartSec=5

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/gex
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX AF_NETLINK

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

Il n'y en a d'ailleurs pas besoin : un tunnel sortant rend la même chose sans
rien ouvrir. C'est **Cloudflare Tunnel** qui a été retenu, et il tourne.
Tailscale est décrit d'abord parce qu'il reste le choix le plus sûr — et parce
que ce qui l'a écarté ici est un besoin précis, pas un défaut.

### Tailscale : le plus sûr, mais il faut l'installer partout

Tailscale monte un réseau privé entre tes appareils — le Pi, ton portable, ton
téléphone. Aucune redirection de port, aucun DNS dynamique à entretenir, aucun
certificat. Gratuit pour un usage personnel, paquet ARM64 disponible.

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

### Cloudflare Tunnel — ce qui est en place

C'est la solution retenue, et elle tourne : l'écran est sur
**`https://gex.unfiltr.app`**.

`cloudflared` monte un tunnel **sortant** — le Pi se connecte à Cloudflare, rien
n'entre — et rend une adresse en HTTPS joignable depuis n'importe quel appareil,
y compris ceux sur lesquels on ne peut rien installer. C'est ce dernier point qui
l'a emporté sur Tailscale.

Le montage tient en trois commandes, une fois `cloudflared` installé depuis le
dépôt officiel de Cloudflare :

```sh
cloudflared tunnel login          # ouvre un navigateur, autorise la zone
cloudflared tunnel create gex-pi
cloudflared tunnel route dns gex-pi gex.unfiltr.app
```

La configuration n'a qu'une route, et le refus explicite qui la suit compte
autant qu'elle :

```yaml
# /etc/cloudflared/config.yml
tunnel: <uuid>
credentials-file: /etc/cloudflared/<uuid>.json

ingress:
  - hostname: gex.unfiltr.app
    service: http://127.0.0.1:8787
  - service: http_status:404
```

Le `http_status:404` final garantit que le tunnel ne peut servir de porte vers
rien d'autre sur cette machine — et surtout pas vers le 4001 de Gateway. Un
tunnel sans cette règle de fermeture est un tunnel dont on ne connaît pas la
surface.

L'unité systemd porte `--no-autoupdate` : le paquet vient d'apt, et sans ce
drapeau `cloudflared` se remplace tout seul puis se redémarre à un moment qu'on
n'a pas choisi.

#### L'adresse est publique, et c'est un choix

**Il n'y a pas de Cloudflare Access devant.** L'écran n'ayant aucune
authentification à lui, quiconque connaît l'adresse lit les niveaux — spot, murs,
zéro gamma. Le choix a été fait en connaissance de cause ; il se révoque en deux
clics dans le tableau de bord Cloudflare, sans rien retoucher sur le Pi.

Ce que cela n'expose pas, en revanche, mérite d'être dit : le tunnel ne mappe que
le 8787, `nftables` ferme le 4001, et la session Gateway reste injoignable
depuis l'extérieur comme depuis le réseau local.

## Mettre à jour sans recompiler

Deux workflows vivent dans `.github/workflows`.

**`ci.yml`** rejoue les tests sur aarch64 à chaque poussée, et vérifie le
formatage. C'est tout ce qu'il fait, et c'est délibéré : il exécute vraiment la
suite sur l'architecture du Pi, ce que la machine de développement ne sait pas
faire. Les tests x86 et clippy se lancent à la main avant de pousser — les
rejouer ici n'aurait rien appris.

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

L'accès à distance et le pare-feu se sont ajoutés à la liste des choses faites.
Restent :

1. **Enregistrer la ligne d'historique quotidienne.** Le collecteur écrit les
   parquet ; c'est le binaire `gex` qui ajoute la ligne à `history.csv`, et rien
   ne le lance sur le Pi. Les relevés s'accumulent donc, mais pas les
   observations qui alimentent les six affirmations — et une séance non
   enregistrée est perdue pour toujours. Un timer systemd après la clôture
   suffirait.
2. **Confirmer qu'une séance entière tient sans redémarrage.** Le Pi s'est déjà
   figé une fois — alimenté mais sourd, sans trace au journal parce qu'il était
   volatil. Le journal est désormais persistant, `panic=10` est passé au noyau et
   une sentinelle surveille les écritures disque ; il faut maintenant du temps
   pour savoir si cela suffit.
3. **Remonter les alertes**, en écrivant `/etc/gex.env`. Deux lignes, aucun
   redéploiement.

Les tests sur l'architecture, eux, n'ont plus à être refaits à la main : la CI les
rejoue en aarch64 à chaque poussée.
