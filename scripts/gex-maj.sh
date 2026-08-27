#!/bin/sh
# Met à jour les binaires du Pi depuis la dernière version publiée.
#
# Le Pi ne compile pas et ne reçoit rien : il va CHERCHER. Aucun port entrant,
# aucun runner sur la machine qui tient la session Gateway — c'est ce qui permet
# de garder le dépôt public sans exposer la machine de collecte.
set -eu

DEPOT=TazTheworld/Option-Gamma-Exposure--zero-gamma-level
ARCHIVE=gex-aarch64.tar.gz
DEST=${GEX_BIN:-/opt/gex/bin}
BASE=https://github.com/$DEPOT/releases/latest/download

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"

echo "Téléchargement de la dernière version..."
curl -fsSL -O "$BASE/$ARCHIVE"
curl -fsSL -O "$BASE/$ARCHIVE.sha256"

# Vérifiée AVANT de toucher à quoi que ce soit : un téléchargement tronqué
# remplacerait sinon un collecteur qui fonctionne par un fichier mort, et on ne
# s'en apercevrait qu'au redémarrage.
echo "Vérification de la somme de contrôle..."
sha256sum -c "$ARCHIVE.sha256"
tar xzf "$ARCHIVE"

# L'arrêt coûte le balayage en cours — une quinzaine de minutes avant que la
# chaîne soit de nouveau complète. Mieux vaut donc mettre à jour hors séance.
echo "Arrêt des services..."
sudo systemctl stop gex-collector gex-web

sudo install -m 755 gex gex-collector gex-web "$DEST"

echo "Redémarrage..."
sudo systemctl start gex-collector gex-web
sudo systemctl --no-pager --lines=0 status gex-collector gex-web
