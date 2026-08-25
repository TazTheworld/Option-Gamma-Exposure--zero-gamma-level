"""Gamma Exposure (GEX) et Zero Gamma Level à partir des données CBOE gratuites.

Ce fichier n'est plus qu'une interface : le calcul est dans analysis.py, les
greeks dans greeks.py, les graphiques dans plots.py, l'archivage dans
snapshots.py. Tout ce qui produit un chiffre est donc testable sans réseau.

Usage :
    python main.py SPCX
    python main.py _SPX --dte-max 7
    python main.py --csv spx_quotedata.csv
    python main.py _SPX --replay snapshots/SPX/2026-08-12_1436.parquet --dte-max 7
"""

import argparse
import os

import pandas as pd

import analysis
import cboe_data
import plots
import snapshots
from cme_data import CONTRACT_SIZES
from greeks import CONTRACT_SIZE

pd.options.display.float_format = "{:,.4f}".format


def dte_arg(value):
    """Type argparse pour --dte-max : un entier de jours, ou 'all' pour ne pas filtrer."""
    if value.lower() in ("all", "toutes", "none"):
        return None
    try:
        days = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError(f"nombre de jours ou 'all' attendu, reçu {value!r}")
    if days < 0:
        raise argparse.ArgumentTypeError("--dte-max doit être positif, ou 'all'")
    return days


def construire_parser():
    parser = argparse.ArgumentParser(description="Gamma Exposure / Zero Gamma Level (données CBOE)")
    parser.add_argument("ticker", nargs="?", default="SPCX",
                        help="ticker du sous-jacent (SPCX, TSLA, _SPX...)")
    parser.add_argument("--csv", help="utiliser un export CSV CBOE au lieu de l'API JSON")
    parser.add_argument("--cme", help="export de règlement CME (options sur futures, ex. 6E)")
    parser.add_argument("--databento", action="store_true",
                        help="récupérer la chaîne CME via Databento (DATABENTO_API_KEY)")
    parser.add_argument("--replay", metavar="FICHIER",
                        help="rejouer un relevé archivé (voir python snapshots.py)")
    parser.add_argument("--suivre", action="store_true",
                        help="lire le relevé courant écrit par le collecteur "
                             "(snapshots/<TICKER>/courant.parquet)")
    parser.add_argument("--dir", default=snapshots.DOSSIER,
                        help=f"dossier des relevés archivés (défaut : {snapshots.DOSSIER})")
    parser.add_argument("--date", help="séance à charger AAAA-MM-JJ (défaut : dernière close)")
    parser.add_argument("--futures-price", type=float,
                        help="prix du future ; déduit par parité call-put si omis")
    parser.add_argument("--expiry", help="échéance AAAA-MM-JJ, si absente du fichier CME")
    parser.add_argument("--quote-date", help="date de valorisation AAAA-MM-JJ (défaut : aujourd'hui)")
    parser.add_argument("--outdir", default="charts", help="dossier de sortie des graphiques")
    parser.add_argument("--no-show", action="store_true", help="enregistrer sans ouvrir les fenêtres")
    parser.add_argument("--no-charts", action="store_true", help="ne produire aucun graphique")
    parser.add_argument("--range", type=float, default=0.2,
                        help="demi-plage de strikes autour du spot (0.2 = +/-20%%)")
    parser.add_argument("--dte-max", type=dte_arg, default=30, metavar="N",
                        help="ne garder que les échéances à N jours calendaires ou moins "
                             "(défaut : 30 ; 'all' pour toute la chaîne)")
    parser.add_argument("--dte-min", type=int, default=0, metavar="N",
                        help="exclure les échéances à moins de N jours (0DTE instables "
                             "avec des données différées ; essayer 2)")
    parser.add_argument("--gamma-source", choices=analysis.SOURCES_GAMMA, default="iv",
                        help="gamma recalculé depuis l'IV (défaut, cohérent avec le profil) "
                             "ou tel que publié par la source")
    parser.add_argument("--time-convention", choices=analysis.CONVENTIONS_TEMPS,
                        default="heures",
                        help="mesure du temps restant : 'heures' (réel, convention CBOE) "
                             "ou 'bourse' (jours ouvrés/262, convention Perfiliev)")
    parser.add_argument("--vol-regime", choices=analysis.REGIMES_VOL,
                        default="sticky-strike",
                        help="comportement de l'IV dans le profil : 'sticky-strike' "
                             "(défaut, IV figée par contrat) ou 'sticky-moneyness' "
                             "(l'IV suit la monnaie — plus réaliste à la baisse)")
    parser.add_argument("--wall-range", type=float, default=0.15,
                        help="demi-plage de recherche des murs gamma autour du spot (0.15 = +/-15%%)")
    parser.add_argument("--oi-wall-range", type=float, default=0.30,
                        help="demi-plage des murs en open interest brut (0.30 = +/-30%%) ; "
                             "plus large que les murs gamma, qui restent collés à la monnaie")
    parser.add_argument("--contract-size", type=float,
                        help="multiplicateur du contrat (défaut : 100, ou 125000 avec --cme)")
    parser.add_argument("--history", default="history.csv",
                        help="fichier d'historique des relevés (défaut : history.csv)")
    parser.add_argument("--no-history", action="store_true",
                        help="ne pas enregistrer ce relevé dans l'historique")
    parser.add_argument("--snapshot-dir", default=snapshots.DOSSIER,
                        help=f"dossier d'archivage des chaînes brutes (défaut : {snapshots.DOSSIER})")
    parser.add_argument("--no-snapshot", action="store_true",
                        help="ne pas archiver la chaîne brute de ce relevé")
    parser.add_argument("--watch", metavar="INTERVALLE",
                        help="échantillonner en boucle pendant la séance (5m, 300, 1h) : "
                             "l'open interest ne bouge qu'une fois par jour, mais le spot "
                             "et l'IV oui, donc le zero gamma dérive en séance")
    parser.add_argument("--watch-duration", metavar="DUREE", default="6h",
                        help="durée totale du suivi (défaut : 6h)")
    return parser


def source_relecture(args):
    """Le fichier à rejouer : --replay tel quel, ou le courant si --suivre."""
    if args.replay:
        return args.replay
    if args.suivre:
        return snapshots.courant(args.ticker, args.dir)
    return None


def verifier_exclusions(args):
    """Les combinaisons d'options qui n'ont pas de sens, refusées à l'entrée.

    --watch relit sa source à intervalle régulier. Sur une archive horodatée
    c'est absurde : elle ne bougera plus. Sur le relevé courant c'est l'usage
    même, puisque le collecteur le réécrit toutes les quinze secondes.
    L'exclusion porte donc sur --replay, jamais sur --suivre.
    """
    if args.watch and args.replay:
        raise ValueError("--watch et --replay s'excluent : une archive ne bouge plus. "
                         "Pour suivre un relevé vivant, utilise --suivre.")
    if args.replay and args.suivre:
        raise ValueError("--replay et --suivre s'excluent : choisis une archive "
                         "précise, ou le relevé courant.")


def charger(args):
    """Renvoie (df, spot, quote_date, ticker, rejoue, marche).

    `marche` est le contexte de séance du sous-jacent (OHLCV, iv30). Seul le CBOE
    le publie : les sources sur futures renvoient un dictionnaire vide, et les
    ratios au volume sont alors simplement absents plutôt que faux.
    """
    source = source_relecture(args)
    if source:
        df, spot, quote_date, marche = snapshots.charger(source)
        return df, spot, quote_date, args.ticker, True, marche
    if args.databento:
        import databento_data
        df, spot, quote_date = databento_data.fetch_chain(args.ticker, day=args.date)
        return df, args.futures_price or spot, quote_date, args.ticker, False, {}
    if args.cme:
        import cme_data
        df, spot, quote_date = cme_data.load_settlement(
            args.cme, futures_price=args.futures_price, expiry=args.expiry,
            quote_date=args.quote_date, product=args.ticker)
        return df, spot, quote_date, args.ticker, False, {}
    if args.csv:
        df, spot, quote_date = cboe_data.load_from_csv(args.csv)
        return df, spot, quote_date, args.ticker, False, {}
    df, spot, quote_date, marche = cboe_data.fetch_chain_et_marche(args.ticker)
    return df, spot, quote_date, cboe_data.cboe_symbol(args.ticker).lstrip("_"), False, marche


def afficher(a):
    """Écrit le rapport de séance sur la sortie standard."""
    dec = a.decimals
    print(f"{a.ticker} | sous-jacent {a.spot:,.{dec}f} "
          f"| {a.df.StrikePrice.nunique()} strikes / {a.df.ExpirationDate.nunique()} "
          f"échéances ({a.horizon}) | {a.quote_date:%Y-%m-%d} "
          f"| contrat x{a.contract_size:,.0f} | gamma {a.source_gamma} | T {a.time_convention} | vol {a.regime_vol}")

    scale, unit = analysis.pick_scale([a.total_gex])
    gscale, gunit = analysis.pick_scale([a.total_charm, a.total_vanna])

    def fmt(x):
        return f"{x:,.{dec}f}" if x is not None else "n/a"

    print(f"Total GEX  : {a.total_gex / scale:,.2f} {unit} $ / mouvement de 1%")
    print(f"Zero Gamma : {fmt(a.zero_gamma)}")
    print(f"Call Wall  : {fmt(a.call_wall):>12} (gamma)   {fmt(a.call_wall_oi):>12} (open interest)")
    print(f"Put Wall   : {fmt(a.put_wall):>12} (gamma)   {fmt(a.put_wall_oi):>12} (open interest)")
    # Charm : delta que les dealers doivent racheter (négatif) ou revendre (positif)
    # pour chaque jour qui passe, à prix inchangé.
    print(f"Charm      : {a.total_charm / gscale:+,.2f} {gunit} $ de delta / jour")
    print(f"Vanna      : {a.total_vanna / gscale:+,.2f} {gunit} $ de delta / point de vol")

    # Le GEX nu ne dit pas s'il pèse quelque chose. Rapporté au volume échangé du
    # jour, si : sous quelques pour cent c'est un frottement, au-delà du tiers la
    # couverture des dealers est un acteur majeur du carnet.
    ratio = a.gex_sur_volume
    if ratio is not None:
        vscale, vunit = analysis.pick_scale([a.marche["dollar_volume"]])
        print(f"GEX/volume : {ratio:.1%} du volume du jour "
              f"({a.marche['dollar_volume'] / vscale:,.2f} {vunit} $ échangés)")
    if (a.marche or {}).get("iv30") is not None:
        print(f"IV 30j     : {a.marche['iv30']:,.1f}%")

    avertir(a)


def avertir(a):
    """Les trois diagnostics qui décident si les chiffres ci-dessus sont lisibles."""
    if a.zero_gamma is None:
        print("\nAttention : pas de changement de signe du gamma dans la plage analysée "
              f"({a.from_strike:,.2f} - {a.to_strike:,.2f}) — élargis avec --range, "
              "ou allonge l'horizon avec --dte-max.")
    elif len(a.croisements) > 1:
        autres = ", ".join(f"{x:,.{a.decimals}f}" for x in a.croisements
                           if abs(x - a.zero_gamma) > 1e-9)
        print(f"\nAttention : le profil croise zéro {len(a.croisements)} fois "
              f"(également en {autres}). Le niveau retenu est\nle plus proche du spot ; "
              "le régime n'est pas une simple bascule au-dessus / en dessous.")

    # Le gamma publié par la source et celui recalculé depuis l'IV s'accordent bien
    # au-delà de quelques jours, et divergent violemment près de l'échéance : le gamma
    # y explose et dépend du spot à la minute, que des données différées ne donnent pas.
    if a.ecart_gamma:
        iv, publie, relatif = a.ecart_gamma
        # Avec --gamma-source published on le signale toujours : le profil, lui, ne
        # peut être que recalculé depuis l'IV, donc le Total GEX et le Zero Gamma
        # ci-dessus ne viennent pas du même estimateur. C'était le cas en
        # permanence et sans le dire ; c'est maintenant un choix explicite.
        if a.source_gamma == "published":
            print(f"\nNote : le Total GEX suit le gamma publié, mais le Zero Gamma vient "
                  f"forcément du gamma\nrecalculé depuis l'IV — aucun gamma publié "
                  f"n'existe à un niveau de spot hypothétique.\nÉcart entre les deux "
                  f"estimateurs sur ce périmètre : {relatif:+.1%}.")
        elif abs(relatif) > 0.05:
            scale, unit = analysis.pick_scale([iv, publie])
            print(f"\nAttention : le gamma recalculé depuis l'IV donne "
                  f"{iv / scale:+,.2f} {unit} $ de GEX,\net le gamma publié par la source "
                  f"{publie / scale:+,.2f} — soit {relatif:+.0%} d'écart. Les deux "
                  f"lectures sont\ndisponibles via --gamma-source ; l'écart se creuse "
                  "sur les échéances courtes.")

    if a.part_courtes:
        pire = max(a.part_courtes.values())
        if pire > 0.20:
            detail = ", ".join(f"{p:.0%} {nom}" for nom, p in a.part_courtes.items())
            print(f"\nAttention : les échéances à 0-1 jour portent {detail}. Leurs greeks "
                  f"sont instables\nsur des données différées — compare avec --dte-min 2 "
                  "avant de conclure.\n")


def enregistrer_historique(args, a):
    """Ajoute ce relevé à l'historique.

    Une ligne par relevé : sans ça chaque analyse est un instantané, et les séries
    n'existent nulle part. Le périmètre d'échéance, la source de gamma et la
    convention de temps sont enregistrés avec — deux relevés qui n'en partagent
    pas ne sont pas comparables, l'écart entre sources dépassant 10 % sur un indice.
    """
    import history

    marche = a.marche or {}
    return history.record(
        args.history, timestamp=a.quote_date, ticker=a.ticker,
        dte_max="all" if a.dte_max is None else a.dte_max,
        source_gamma=a.source_gamma, time_convention=a.time_convention,
        gex_sur_volume=a.gex_sur_volume,
        **{colonne: marche.get(history.MARCHE_ALIAS.get(colonne, colonne))
           for colonne in ("open", "high", "low", "close", "prev_close",
                           "volume", "dollar_volume", "iv30")},
        spot=a.spot, total_gex=a.total_gex, zero_gamma=a.zero_gamma,
        call_wall=a.call_wall, put_wall=a.put_wall,
        call_wall_oi=a.call_wall_oi, put_wall_oi=a.put_wall_oi,
        charm=a.total_charm, vanna=a.total_vanna,
        strikes=a.df.StrikePrice.nunique(),
        expiries=a.df.ExpirationDate.nunique())


def un_passage(args, contract_size, graphiques=True, historique=True):
    """Un relevé complet : chargement, archivage, analyse, affichage, historique."""
    df, spot, quote_date, ticker, rejoue, marche = charger(args)

    # L'archivage vient avant le filtre d'échéance : c'est la chaîne BRUTE qu'on
    # garde, pour pouvoir rejouer la séance à n'importe quel horizon plus tard.
    if not (args.no_snapshot or rejoue):
        archive = snapshots.sauver(df, ticker, spot, quote_date, args.snapshot_dir, marche)
        print(f"Chaîne brute archivée : {archive}")

    a = analysis.analyser(
        df, spot=spot, quote_date=quote_date, ticker=ticker, contract_size=contract_size,
        dte_max=args.dte_max, dte_min=args.dte_min, plage=args.range,
        wall_range=args.wall_range, oi_wall_range=args.oi_wall_range,
        source_gamma=args.gamma_source, time_convention=args.time_convention,
        regime_vol=args.vol_regime, marche=marche)
    afficher(a)

    if historique and not args.no_history:
        enregistrer_historique(args, a)

    if graphiques and not args.no_charts:
        plots.tracer(a, args.outdir)
        print(f"Graphiques enregistrés dans : {os.path.abspath(args.outdir)}")
        if not args.no_show:
            plots.afficher()
    return a


def suivre(args, contract_size):
    """Échantillonne la séance en boucle, un relevé par passage.

    L'open interest ne bouge qu'une fois par jour : en séance, seuls le spot et
    la volatilité implicite changent. Le zero gamma ne se déplace donc pas
    beaucoup, mais la DISTANCE du prix à ce niveau, elle, se referme ou s'ouvre —
    et c'est elle qui décide du régime dans lequel on se trouve.

    Un passage n'écrit dans l'historique que si le flux a réellement avancé : les
    données CBOE sont différées d'un quart d'heure, donc deux passages rapprochés
    renvoient le même relevé, et le réenregistrer n'ajouterait qu'une ligne
    identique. validate.py ne retient de toute façon qu'une séance par jour, la
    dernière : ces relevés servent à voir la journée, pas à gonfler l'échantillon.
    """
    import time

    from flow_tracker import duree

    intervalle = duree(args.watch)
    fin = time.time() + duree(args.watch_duration)
    print(f"Suivi de {args.ticker} toutes les {intervalle}s pendant "
          f"{duree(args.watch_duration) / 3600:.1f}h — Ctrl+C pour arrêter.\n")

    passage = enregistres = 0
    precedent = None
    while True:
        passage += 1
        try:
            # L'enregistrement se décide APRÈS le relevé : on ne connaît
            # l'horodatage du flux qu'une fois la chaîne téléchargée.
            a = un_passage(args, contract_size, graphiques=False, historique=False)
        except (ValueError, OSError) as err:
            print(f"  relevé manqué ({type(err).__name__} : {err}) — on continue")
            a = None

        if a is not None:
            avance = precedent is None or a.quote_date != precedent.quote_date
            if avance and not args.no_history:
                enregistrer_historique(args, a)
                enregistres += 1
            elif not avance:
                print("  (flux inchangé depuis le relevé précédent — rien à enregistrer)")

            if precedent is not None:
                ecart_spot = a.spot - precedent.spot
                ecart_zg = ((a.zero_gamma - precedent.zero_gamma)
                            if None not in (a.zero_gamma, precedent.zero_gamma) else None)
                distance = (a.spot - a.zero_gamma) / a.spot if a.zero_gamma else None
                print(f"  depuis le relevé précédent : spot {ecart_spot:+,.{a.decimals}f}"
                      + (f", zero gamma {ecart_zg:+,.{a.decimals}f}" if ecart_zg is not None else "")
                      + (f", distance au zero gamma {distance:+.2%}" if distance is not None else ""))
            precedent = a

        reste = fin - time.time()
        if reste < intervalle / 2:      # pas de fenêtre tronquée en fin de course
            break
        print()
        time.sleep(min(intervalle, reste))

    print(f"\n{passage} relevés, {enregistres} enregistrés dans {args.history} "
          f"(les autres portaient un flux inchangé).")
    return precedent


def main(argv=None):
    args = construire_parser().parse_args(argv)

    # Les options sur futures ont un multiplicateur tout autre que les actions
    contract_size = args.contract_size
    if contract_size is None:
        futures = args.cme or args.databento
        contract_size = CONTRACT_SIZES.get(args.ticker.upper(), 125_000) if futures else CONTRACT_SIZE

    verifier_exclusions(args)
    if args.watch:
        return suivre(args, contract_size)
    return un_passage(args, contract_size)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as err:
        raise SystemExit(f"Erreur : {err}")
