#!/bin/sh
# Remet une alerte du collecteur a Discord.
#
# Le collecteur ne connait pas Discord : il forme une charge et l'ecrit sur
# l'entree standard de ce script. Changer de destination ne demande donc pas de
# recompiler quoi que ce soit - il suffit d'un autre script.
#
#   export GEX_DISCORD_WEBHOOK=https://discord.com/api/webhooks/<id>/<jeton>
#   export GEX_ALERTE_COMMANDE=/opt/gex/bin/alerte-discord.sh
#   gex-collector NQ
#
# --max-time n'est pas une precaution de style : le collecteur ATTEND la fin de
# ce script, et un envoi qui ne rendrait jamais la main figerait la collecte.
set -eu

: "${GEX_DISCORD_WEBHOOK:?definir GEX_DISCORD_WEBHOOK}"

exec curl -sS --fail --max-time 10 \
    -X POST -H 'Content-Type: application/json' \
    --data-binary @- \
    "$GEX_DISCORD_WEBHOOK"
