#!/bin/sh
# Depose les deux rapports CFTC que `gex --cftc` confronte.
#
# Le lecteur ne va JAMAIS chercher de donnees - c'est ce qui fait qu'une seance
# passee se rejoue avec le meme code qu'une seance vivante. Le reseau est donc
# ici, et nulle part ailleurs.
#
#   ./scripts/cftc-telecharger.sh          depose dans ./cftc
#   ./scripts/cftc-telecharger.sh ailleurs depose dans ./ailleurs
#
# Les deux fichiers ont exactement la meme forme : c'est le champ
# `futonly_or_combined` qui les distingue, et `gex --cftc` refuse de les
# confondre. Les intervertir rendrait une difference nulle, donc un « delta
# d'options toujours nul » parfaitement credible et faux.
set -eu

DEST=${1:-cftc}
CONTRAT=${CFTC_CONTRAT:-NASDAQ MINI}
BASE=https://publicreporting.cftc.gov/resource
CHAMPS='report_date_as_yyyy_mm_dd,contract_market_name,futonly_or_combined,dealer_positions_long_all,dealer_positions_short_all,open_interest_all'

mkdir -p "$DEST"

telecharger() {
    jeu=$1
    nom=$2
    curl -sS --fail --max-time 120 -G "$BASE/$jeu.json" \
        --data-urlencode "\$where=contract_market_name='$CONTRAT'" \
        --data-urlencode "\$select=$CHAMPS" \
        --data-urlencode '$order=report_date_as_yyyy_mm_dd DESC' \
        --data-urlencode '$limit=5000' \
        -o "$DEST/$nom"
    echo "  $nom : $(wc -c < "$DEST/$nom") octets"
}

echo "Contrat : $CONTRAT"
# yw9f-hn96 = futures ET options combines ; gpe5-46if = futures seuls.
# Leur difference est le delta net du livre d'options des teneurs.
telecharger yw9f-hn96 combine.json
telecharger gpe5-46if futures.json
echo
echo "Deposes dans $DEST. Confronter avec :"
echo "  gex --cftc"
