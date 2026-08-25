"""Collecteur Interactive Brokers : socle au réveil, vif entretenu, relevé courant.

    python ib_collector.py NQ                      # tout l'horizon, 30 jours
    python ib_collector.py NQ --dte-max 7          # plus rapide au démarrage
    python ib_collector.py NQ --range 0.05         # et plus étroit encore

Le socle est un balayage complet du périmètre, refait une fois par journée de
compensation — l'open interest est calculé par la chambre après la clôture et
publié une fois par jour, le relire en séance coûterait huit minutes pour le même
chiffre. Le vif entretient quatre-vingt-dix souscriptions sur les contrats qui
portent le plus de gamma, et rafraîchit l'IV et le prix du future en continu.

Par défaut le collecteur prend TOUT ce qu'IB liste sur l'horizon, sans filtre de
strike. Filtrer n'économise pas grand-chose — de ±20 % à ±5 %, on ne retire que
vingt-sept pour cent des contrats, la grille étant dense près de la monnaie et
clairsemée au large — alors qu'une donnée non collectée est perdue pour toujours.
`main.py` filtre au calcul, avec --range et --dte-max.

Le relevé courant est réécrit toutes les quinze secondes à chemin fixe, l'archive
horodatée toutes les quinze minutes. `python main.py NQ --suivre` lit le premier.

Le calcul n'est pas ici : ce module produit une chaîne, analysis.analyser() en
tire les chiffres. Les dupliquer créerait deux endroits où la même formule
pourrait diverger — ce que le dépôt a mis plusieurs commits à défaire.

Conception : docs/superpowers/specs/2026-08-25-collecteur-ib-nq-design.md
"""

import argparse
import os

import pandas as pd

import ib_data
import snapshots

# Le CME publie l'open interest préliminaire à 18 h heure de Chicago, soit 23 h
# UTC en heure d'été. Le socle est donc rebalayé au premier réveil qui suit, et
# une seule fois par journée de compensation.
HEURE_OI_UTC = 23

RAFRAICHIR_DEFAUT = 15        # secondes entre deux écritures du courant
ARCHIVER_DEFAUT = 15 * 60     # secondes entre deux archives horodatées
MARGE_BANDE = 0.15            # part de la bande au-delà de laquelle on recycle


# ---=== Les décisions — pures, donc testées ===---

def journee_compensation(instant):
    """La journée d'open interest à laquelle un instant appartient.

    Après 23 h UTC, on est déjà sur la publication du lendemain.
    """
    ts = pd.Timestamp(instant)
    return (ts + pd.Timedelta(hours=24 - HEURE_OI_UTC)).normalize()


def faut_il_rebalayer(dernier_socle, maintenant):
    """Le socle est-il périmé ?

    L'open interest est calculé par la chambre de compensation après la clôture
    et publié une fois par jour : rebalayer en cours de journée relirait le même
    chiffre pour huit minutes de souscriptions.
    """
    if dernier_socle is None:
        return True
    return journee_compensation(maintenant) > journee_compensation(dernier_socle)


def bande_couverte(vif):
    """Les strikes extrêmes que le vif surveille, ou None s'il est vide."""
    if vif is None or len(vif) == 0:
        return None
    strikes = pd.to_numeric(vif["StrikePrice"], errors="coerce").dropna()
    if strikes.empty:
        return None
    return float(strikes.min()), float(strikes.max())


def faut_il_reselectionner(spot, bande, marge=MARGE_BANDE):
    """Le spot approche-t-il du bord de ce que le vif surveille ?

    Attendre la sortie franche reviendrait à attendre d'être aveugle : quand le
    spot arrive près du bord, les contrats qui portent le gamma ne sont déjà plus
    ceux qu'on a souscrits.
    """
    if bande is None:
        return True
    bas, haut = bande
    largeur = haut - bas
    if largeur <= 0:
        return True
    garde = largeur * marge
    return not (bas + garde <= float(spot) <= haut - garde)


# ---=== La boucle — exige TWS, donc vérifiée à la main ===---

def _ecrire_courant(df, spot, quote_date, chemin):
    """Écrit le relevé vivant au chemin fixe, format identique aux archives.

    snapshots.sauver() ne convient pas ici : il horodate à la minute, et l'appeler
    toutes les quinze secondes écrirait quatre fichiers par minute.
    """
    os.makedirs(os.path.dirname(chemin), exist_ok=True)
    out = df.copy()
    out[snapshots.META_SPOT] = float(spot)
    out[snapshots.META_DATE] = pd.Timestamp(quote_date)
    if chemin.endswith(".parquet"):
        out.to_parquet(chemin, index=False)
    else:
        out.to_csv(chemin, index=False, compression="gzip")
    return chemin


def _spot_amorce(ib, futur):
    """Prix du future au tout premier balayage, avant qu'aucun tick n'existe.

    Les balayages suivants n'en ont pas besoin : undPrice arrive avec les grecs
    de chaque option.
    """
    ticker = ib.reqMktData(futur, "", False, False)
    ib.sleep(4)
    valeur = None
    for champ in ("last", "close", "bid"):
        candidat = ib_data._prix(getattr(ticker, champ, None))
        if candidat == candidat:          # non NaN
            valeur = candidat
            break
    ib.cancelMktData(futur)
    if valeur is None:
        raise ValueError(
            "Prix du future indéterminable au démarrage. Marché fermé, ou "
            "données de marché indisponibles ?"
        )
    return valeur


def _ticks_du_vif(ib, vif_contrats):
    """Relit les souscriptions entretenues, sans rien demander de neuf."""
    lignes = [ib_data.ligne_ticker(ib.ticker(ib_data._contrat_ib(l)))
              for l in vif_contrats.itertuples()]
    ticks = pd.DataFrame(lignes)
    # Les contrats du vif portent déjà leur identité : on la recolle par
    # position, l'ordre étant celui de vif_contrats.
    for colonne in ("ExpirationDate", "StrikePrice", "right"):
        ticks[colonne] = vif_contrats[colonne].to_numpy()
    return ticks


def collecter_en_boucle(ticker="NQ", plage=None, dte_max=30, dte_min=0,
                        budget=ib_data.BUDGET_LIGNES, attente=ib_data.ATTENTE_LOT,
                        rafraichir=RAFRAICHIR_DEFAUT, archiver=ARCHIVER_DEFAUT,
                        differe=True, dossier=snapshots.DOSSIER,
                        port=ib_data.PORT_DEFAUT, client_id=ib_data.CLIENT_ID_DEFAUT):
    """La boucle. Ctrl+C pour arrêter.

    `plage=None` prend tout ce qu'IB liste — c'est le défaut, et le filtrage se
    fait au calcul plutôt qu'à la collecte.
    """
    ib = ib_data.connecter(port=port, client_id=client_id, differe=differe)
    futur = ib_data.front_month(ib, ticker)
    print(f"Future {futur.localSymbol} — multiplicateur x{futur.multiplier}")

    socle = spot = vif_contrats = catalogue = None
    date_socle = quote_date = dernier_archive = None

    try:
        while True:
            maintenant = pd.Timestamp.now("UTC").tz_localize(None)

            # --- le socle, une fois par journée de compensation ---
            if faut_il_rebalayer(date_socle, maintenant):
                print(f"\n[{maintenant:%H:%M:%S}] Balayage du socle…")
                tous = ib_data.enumerer(ib, futur, dte_max, dte_min, maintenant)
                if plage is not None:
                    repere = spot if spot else _spot_amorce(ib, futur)
                    tous = ib_data.perimetre(tous, repere, plage)
                n_lots = len(ib_data.lots(tous, budget))
                print(f"  {len(tous):,} contrats, {n_lots} lots "
                      f"— compter {n_lots * (attente + 2.6) / 60:.0f} min")
                ticks = ib_data.collecter(ib, tous, budget, attente)
                socle, spot, quote_date = ib_data.build_chain(tous, ticks)
                # build_chain passe au format large et y perd les conId : chaque
                # ligne porte un call ET un put. Le catalogue les garde, pour que
                # le vif reste souscriptible.
                catalogue = tous
                date_socle = maintenant
                vif_contrats = None
                muets = ticks.OpenInt.isna().sum()
                print(f"  socle : {len(socle)} strikes, future {spot:,.2f}"
                      f" ({muets} contrats sans réponse)")

            # --- le vif ---
            if faut_il_reselectionner(spot, bande_couverte(vif_contrats)):
                if vif_contrats is not None:
                    for ligne in vif_contrats.itertuples():
                        ib.cancelMktData(ib_data._contrat_ib(ligne))
                    ib.sleep(0.5)
                vif_contrats = ib_data.avec_conid(
                    ib_data.selection_vif(socle, budget), catalogue)
                for ligne in vif_contrats.itertuples():
                    ib.reqMktData(ib_data._contrat_ib(ligne),
                                  ib_data.TICKS_GENERIQUES, False, False)
                bas, haut = bande_couverte(vif_contrats)
                print(f"\n[{maintenant:%H:%M:%S}] Vif : {len(vif_contrats)} "
                      f"contrats, bande {bas:,.0f} - {haut:,.0f}")

            ib.sleep(rafraichir)

            # --- fusion et écriture ---
            ticks_vif = _ticks_du_vif(ib, vif_contrats)
            vus = pd.to_numeric(ticks_vif.get("UndPrice"),
                                errors="coerce").dropna()
            if not vus.empty:
                spot = float(vus.median())

            fusionnee, spot = ib_data.fusionner(socle, ticks_vif, spot)
            _ecrire_courant(fusionnee, spot, quote_date,
                            snapshots.courant(ticker, dossier))
            print(f"\r[{maintenant:%H:%M:%S}] future {spot:>10,.2f} — "
                  f"{len(fusionnee)} strikes", end="", flush=True)

            if dernier_archive is None or (
                    maintenant - dernier_archive).total_seconds() >= archiver:
                chemin = snapshots.sauver(fusionnee, ticker, spot, maintenant,
                                          dossier)
                dernier_archive = maintenant
                print(f"\n[{maintenant:%H:%M:%S}] archive : {chemin}")

    except KeyboardInterrupt:
        print("\nArrêt demandé.")
    finally:
        ib.disconnect()
        print("Déconnecté.")


def main():
    p = argparse.ArgumentParser(
        description="Collecteur IB : socle quotidien, vif entretenu")
    p.add_argument("ticker", nargs="?", default="NQ", help="produit CME (NQ, ES…)")
    p.add_argument("--range", type=float, default=None, dest="plage",
                   help="demi-plage de strikes autour du spot (défaut : tout "
                        "prendre ; le filtrage se fait au calcul)")
    p.add_argument("--dte-max", type=int, default=30, metavar="N",
                   help="horizon en jours (défaut : 30 ; 7 démarre bien plus vite)")
    p.add_argument("--dte-min", type=int, default=0, metavar="N",
                   help="exclure les échéances à moins de N jours (1 écarte les "
                        "0DTE, un cinquième des contrats)")
    p.add_argument("--budget", type=int, default=ib_data.BUDGET_LIGNES,
                   help="lignes de données entretenues (100 max sans Quote Booster)")
    p.add_argument("--attente", type=float, default=ib_data.ATTENTE_LOT,
                   help="délai de garde par lot, en secondes")
    p.add_argument("--rafraichir", type=int, default=RAFRAICHIR_DEFAUT,
                   help="secondes entre deux écritures du relevé courant")
    p.add_argument("--archiver", type=int, default=ARCHIVER_DEFAUT,
                   help="secondes entre deux archives horodatées")
    p.add_argument("--temps-reel", action="store_true",
                   help="exiger le temps réel (défaut : différé, qui suffit et "
                        "ne demande aucun abonnement)")
    p.add_argument("--dir", default=snapshots.DOSSIER)
    p.add_argument("--port", type=int, default=ib_data.PORT_DEFAUT)
    p.add_argument("--client-id", type=int, default=ib_data.CLIENT_ID_DEFAUT)
    args = p.parse_args()

    collecter_en_boucle(
        ticker=args.ticker, plage=args.plage, dte_max=args.dte_max,
        dte_min=args.dte_min, budget=args.budget, attente=args.attente,
        rafraichir=args.rafraichir, archiver=args.archiver,
        differe=not args.temps_reel, dossier=args.dir, port=args.port,
        client_id=args.client_id)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as err:
        raise SystemExit(f"Erreur : {err}")
