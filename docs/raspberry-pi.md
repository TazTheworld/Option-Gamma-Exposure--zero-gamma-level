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

## Ce qui reste à faire sur place

1. Installer Gateway ARM64 et vérifier qu'il se connecte, avec `Xvfb` et `IBC`.
2. Autoriser l'API dans Gateway, et y déclarer l'adresse du Pi si Gateway tourne
   ailleurs (`--adresse` côté collecteur pointe alors vers cette machine).
3. Rejouer les tests sur la cible : `cargo test --workspace`.
4. Confirmer qu'une séance entière tient sans redémarrage, et relever la mémoire
   réellement prise par Gateway.
