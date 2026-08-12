"""Gamma Exposure (GEX) et Zero Gamma Level à partir des données CBOE gratuites.

Usage :
    python main.py SPCX
    python main.py _SPX --outdir charts
    python main.py --csv spx_quotedata.csv
"""

import argparse
import os

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
from scipy.stats import norm

import cboe_data
from cme_data import CONTRACT_SIZES

pd.options.display.float_format = "{:,.4f}".format

CONTRACT_SIZE = 100          # actions et ETF US ; 125 000 pour le 6E (Euro FX)
TRADING_DAYS = 262


def calc_gamma_ex(S, K, vol, T, r, q, opt_type, OI, contract_size=CONTRACT_SIZE):
    """Gamma Black-Scholes ($ par mouvement de 1% du sous-jacent), vectorisé.

    S peut être un scalaire ou un vecteur de niveaux de spot ; K, vol, T et OI
    sont des vecteurs de même longueur (une entrée par contrat).

    Avec r = q = 0 cette formule est identiquement égale au gamma de Black-76,
    donc elle s'applique telle quelle aux options sur futures (6E, ES...) :
    seule la taille du contrat change.
    """
    S = np.asarray(S, dtype=float)
    K, vol, T, OI = (np.asarray(x, dtype=float) for x in (K, vol, T, OI))

    valid = (T > 0) & (vol > 0) & (K > 0)
    # On neutralise les entrées invalides avant le log/sqrt pour éviter les warnings
    vol_s, T_s, K_s = np.where(valid, vol, 1.0), np.where(valid, T, 1.0), np.where(valid, K, 1.0)

    dp = (np.log(S / K_s) + (r - q + 0.5 * vol_s ** 2) * T_s) / (vol_s * np.sqrt(T_s))
    if opt_type == "call":
        gamma = np.exp(-q * T_s) * norm.pdf(dp) / (S * vol_s * np.sqrt(T_s))
    else:  # gamma identique calls/puts, formule alternative pour recoupement
        dm = dp - vol_s * np.sqrt(T_s)
        gamma = K_s * np.exp(-r * T_s) * norm.pdf(dm) / (S * S * vol_s * np.sqrt(T_s))

    return np.where(valid, OI * contract_size * S * S * 0.01 * gamma, 0.0)


def _d1_d2(S, K, vol, T):
    """d1 et d2 de Black-Scholes à r = q = 0, avec neutralisation des entrées invalides."""
    S = np.asarray(S, dtype=float)
    K, vol, T = (np.asarray(x, dtype=float) for x in (K, vol, T))
    valid = (T > 0) & (vol > 0) & (K > 0)
    vol_s, T_s, K_s = (np.where(valid, x, 1.0) for x in (vol, T, K))
    d1 = (np.log(S / K_s) + 0.5 * vol_s ** 2 * T_s) / (vol_s * np.sqrt(T_s))
    return d1, d1 - vol_s * np.sqrt(T_s), valid, vol_s, T_s


def calc_charm_ex(S, K, vol, T, OI, contract_size=CONTRACT_SIZE):
    """Charm exposure : dollars de delta gagnés par jour de bourse, à prix constant.

    charm = phi(d1) * d2 / (2T). À q = 0 il est identique pour calls et puts
    (le -1 du delta put ne s'écoule pas).

    T étant exprimé en années DE BOURSE (jours ouvrés / 262), la dérivée est par
    année de bourse : on divise donc par TRADING_DAYS et non par 365. Vérifié par
    différence finie sur le delta dollar du book complet.

    Interprétation : un charm exposure positif signifie que le delta du book
    dealer grossit avec le temps, donc qu'il doit vendre pour rester neutre.
    C'est le flux de couverture des derniers jours avant échéance, celui que le
    gamma seul ne montre pas.
    """
    d1, d2, valid, _, T_s = _d1_d2(S, K, vol, T)
    charm = norm.pdf(d1) * d2 / (2 * T_s)
    OI = np.asarray(OI, dtype=float)
    return np.where(valid, OI * contract_size * np.asarray(S, float) * charm / TRADING_DAYS, 0.0)


def calc_vanna_ex(S, K, vol, T, OI, contract_size=CONTRACT_SIZE):
    """Vanna exposure : dollars de delta par point de volatilité implicite.

    vanna = -phi(d1) * d2 / vol, également identique calls et puts à q = 0.

    Interprétation : un vanna exposure positif signifie que le delta du book
    grossit quand la volatilité monte — les dealers vendent alors dans les pics
    de vol. C'est le canal par lequel un choc de volatilité se transmet au spot.
    """
    d1, d2, valid, vol_s, _ = _d1_d2(S, K, vol, T)
    vanna = -norm.pdf(d1) * d2 / vol_s
    OI = np.asarray(OI, dtype=float)
    return np.where(valid, OI * contract_size * np.asarray(S, float) * vanna / 100.0, 0.0)


def is_third_friday(d):
    return d.weekday() == 4 and 15 <= d.day <= 21


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


def pick_scale(values):
    """Choisit l'unité d'affichage (milliards ou millions) selon l'ordre de grandeur."""
    peak = np.max(np.abs(values)) if len(values) else 0
    return (1e9, "milliards") if peak >= 1e9 else (1e6, "millions")


def find_zero_gamma(levels, profile):
    """Interpole le niveau de spot où le gamma total change de signe."""
    crossings = np.where(np.diff(np.sign(profile)))[0]
    if len(crossings) == 0:
        return None
    idx = crossings[0]
    neg_gamma, pos_gamma = profile[idx], profile[idx + 1]
    neg_strike, pos_strike = levels[idx], levels[idx + 1]
    return pos_strike - ((pos_strike - neg_strike) * pos_gamma / (pos_gamma - neg_gamma))


def annotate_levels(ax, spot, ticker, zero_gamma=None, call_wall=None, put_wall=None,
                    decimals=2):
    """Trace les niveaux clés (spot, zero gamma, murs) sur un axe.

    Les xlim doivent déjà être fixées : un niveau situé en dehors de la fenêtre
    est signalé dans la légende plutôt que tracé hors champ.
    """
    lo, hi = ax.get_xlim()

    def draw(value, color, style, name, width=1.0):
        if value is None:
            return
        suffix = "" if lo <= value <= hi else "  (hors plage)"
        ax.axvline(x=value, color=color, lw=width, ls=style,
                   label=f"{name} : {value:,.{decimals}f}{suffix}")

    draw(spot, "r", "-", f"{ticker} Spot", 1.2)
    draw(zero_gamma, "g", "--", "Zero Gamma", 1.6)
    draw(call_wall, "darkorange", ":", "Call Wall")
    draw(put_wall, "purple", ":", "Put Wall")


def main():
    parser = argparse.ArgumentParser(description="Gamma Exposure / Zero Gamma Level (données CBOE)")
    parser.add_argument("ticker", nargs="?", default="SPCX",
                        help="ticker du sous-jacent (SPCX, TSLA, _SPX...)")
    parser.add_argument("--csv", help="utiliser un export CSV CBOE au lieu de l'API JSON")
    parser.add_argument("--cme", help="export de règlement CME (options sur futures, ex. 6E)")
    parser.add_argument("--databento", action="store_true",
                        help="récupérer la chaîne CME via Databento (DATABENTO_API_KEY)")
    parser.add_argument("--date", help="séance à charger AAAA-MM-JJ (défaut : dernière close)")
    parser.add_argument("--futures-price", type=float,
                        help="prix du future ; déduit par parité call-put si omis")
    parser.add_argument("--expiry", help="échéance AAAA-MM-JJ, si absente du fichier CME")
    parser.add_argument("--quote-date", help="date de valorisation AAAA-MM-JJ (défaut : aujourd'hui)")
    parser.add_argument("--outdir", default="charts", help="dossier de sortie des graphiques")
    parser.add_argument("--no-show", action="store_true", help="enregistrer sans ouvrir les fenêtres")
    parser.add_argument("--range", type=float, default=0.2,
                        help="demi-plage de strikes autour du spot (0.2 = +/-20%%)")
    parser.add_argument("--dte-max", type=dte_arg, default=30, metavar="N",
                        help="ne garder que les échéances à N jours calendaires ou moins "
                             "(défaut : 30 ; 'all' pour toute la chaîne)")
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
    args = parser.parse_args()

    # Les options sur futures ont un multiplicateur tout autre que les actions
    contract_size = args.contract_size
    if contract_size is None:
        futures = args.cme or args.databento
        contract_size = CONTRACT_SIZES.get(args.ticker.upper(), 125_000) if futures else CONTRACT_SIZE

    # ---=== CHARGEMENT DES DONNÉES ===---
    if args.databento:
        import databento_data
        df, spot_price, today_date = databento_data.fetch_chain(args.ticker, day=args.date)
        if args.futures_price:
            spot_price = args.futures_price
        ticker = args.ticker
    elif args.cme:
        import cme_data
        df, spot_price, today_date = cme_data.load_settlement(  # noqa: F811
            args.cme, futures_price=args.futures_price, expiry=args.expiry,
            quote_date=args.quote_date, product=args.ticker)
        ticker = args.ticker
    elif args.csv:
        df, spot_price, today_date = cboe_data.load_from_csv(args.csv)
        ticker = args.ticker
    else:
        df, spot_price, today_date = cboe_data.fetch_chain(args.ticker)
        ticker = cboe_data.cboe_symbol(args.ticker).lstrip("_")

    # ---=== FILTRE D'ÉCHÉANCE ===---
    # Les chaînes CBOE portent plusieurs années d'échéances. Sans filtre, les LEAPS
    # — strikes ronds à très gros OI — dominent les murs et tirent le zero gamma,
    # alors qu'ils ne génèrent quasiment aucun flux de hedging à court terme.
    # Le filtre s'applique ici, avant tout calcul, pour que murs et profil de gamma
    # portent sur le même périmètre.
    dte = (df.ExpirationDate - pd.Timestamp(today_date)).dt.days
    kept = dte >= 0          # une échéance passée n'a plus de gamma : elle fausse le GEX par strike
    if args.dte_max is not None:
        kept &= dte <= args.dte_max
    if not kept.any():
        future = dte[dte >= 0]
        if not len(future):
            raise ValueError("aucune échéance future dans les données")
        raise ValueError(f"aucune échéance à {args.dte_max} jours ou moins "
                         f"(la plus proche est à {int(future.min())} jours) — "
                         f"élargis avec --dte-max {int(future.min())}, ou --dte-max all")
    df = df[kept].copy()

    from_strike = (1 - args.range) * spot_price
    to_strike = (1 + args.range) * spot_price
    decimals = 4 if spot_price < 10 else 2   # les paires FX se lisent en pips
    horizon = "toutes échéances" if args.dte_max is None else f"<= {args.dte_max}j"
    print(f"{ticker} | sous-jacent {spot_price:,.{decimals}f} "
          f"| {df.StrikePrice.nunique()} strikes / {df.ExpirationDate.nunique()} échéances ({horizon}) "
          f"| {today_date:%Y-%m-%d} | contrat x{contract_size:,.0f}")

    # ---=== GAMMA EXPOSURE PAR STRIKE ===---
    # GEX = gamma unitaire * OI * taille du contrat * spot, ramené à un mouvement de 1%
    df["CallGEX"] = df.CallGamma * df.CallOpenInt * contract_size * spot_price ** 2 * 0.01
    df["PutGEX"] = df.PutGamma * df.PutOpenInt * contract_size * spot_price ** 2 * 0.01 * -1
    df["TotalGamma"] = df.CallGEX + df.PutGEX

    df_agg = df.groupby("StrikePrice")[["CallGEX", "PutGEX", "TotalGamma",
                                        "CallOpenInt", "PutOpenInt"]].sum()
    in_range = df_agg[(df_agg.index >= from_strike) & (df_agg.index <= to_strike)]
    strikes = df_agg.index.values

    # Murs de gamma : strikes concentrant le plus de gamma call (résistance) / put (support).
    # La bande est indépendante du zoom d'affichage (--range) mais reste serrée : trop
    # large, elle laisse gagner des strikes ronds lointains dont l'OI est spéculatif.
    #
    # Chaque mur est cherché du bon côté du spot. Sans cette contrainte les deux
    # tombent sur le strike ATM — le gamma unitaire y est maximal, ce qui suffit à
    # battre des strikes dix fois plus chargés en OI — et le résultat n'est alors
    # qu'une paraphrase du spot.
    wall_band = df_agg[(df_agg.index >= (1 - args.wall_range) * spot_price)
                       & (df_agg.index <= (1 + args.wall_range) * spot_price)]
    above = wall_band[wall_band.index >= spot_price]
    below = wall_band[wall_band.index <= spot_price]
    # Un GEX call nul (ou put non négatif) signifie qu'il n'y a pas de mur à retenir :
    # idxmax renverrait alors le premier strike de la bande, ce qui n'a aucun sens.
    call_wall = above.CallGEX.idxmax() if len(above) and above.CallGEX.max() > 0 else None
    put_wall = below.PutGEX.idxmin() if len(below) and below.PutGEX.min() < 0 else None

    # Murs en open interest brut : la lecture « classique », non pondérée par le gamma.
    # Sur une action les deux coïncident souvent, mais sur un indice l'écart est net —
    # le SPX du 12 août donnait 7800/7700 en gamma (soit le spot paraphrasé) contre
    # 8800/6000 en OI. La bande est plus large car ces concentrations sont plus loin.
    oi_band = df_agg[(df_agg.index >= (1 - args.oi_wall_range) * spot_price)
                     & (df_agg.index <= (1 + args.oi_wall_range) * spot_price)]
    oi_above = oi_band[oi_band.index >= spot_price]
    oi_below = oi_band[oi_band.index <= spot_price]
    call_wall_oi = oi_above.CallOpenInt.idxmax() if len(oi_above) and oi_above.CallOpenInt.max() > 0 else None
    put_wall_oi = oi_below.PutOpenInt.idxmax() if len(oi_below) and oi_below.PutOpenInt.max() > 0 else None

    scale, unit = pick_scale(in_range.TotalGamma.values if len(in_range) else df_agg.TotalGamma.values)
    total_gex = df.TotalGamma.sum()

    # Largeur de barre adaptée à l'espacement réel des strikes
    spacing = np.median(np.diff(in_range.index.values)) if len(in_range) > 1 else 1.0
    width = spacing * 0.8

    # ---=== PROFIL DE GAMMA / ZERO GAMMA LEVEL ===---
    levels = np.linspace(from_strike, to_strike, 60)

    # Les 0DTE sont ramenées à 1 jour, sinon elles sortent du calcul (T = 0)
    busdays = np.busday_count(
        np.full(len(df), today_date.date(), dtype="datetime64[D]"),
        df.ExpirationDate.values.astype("datetime64[D]"),
    )
    df["daysTillExp"] = np.where(busdays == 0, 1, busdays) / TRADING_DAYS
    df = df[df.daysTillExp > 0]

    next_expiry = df.ExpirationDate.min()
    third_fridays = df[[is_third_friday(x) for x in df.ExpirationDate]]
    next_monthly_exp = third_fridays.ExpirationDate.min() if len(third_fridays) else None

    masks = {
        "All Expiries": np.ones(len(df), dtype=bool),
        "Ex-Next Expiry": (df.ExpirationDate != next_expiry).values,
    }
    # Avec un --dte-max serré il peut ne rester aucun 3e vendredi : la courbe
    # serait alors identique à "All Expiries" et se superposerait à elle.
    if next_monthly_exp is not None:
        masks["Ex-Next Monthly Expiry"] = (df.ExpirationDate != next_monthly_exp).values
    profiles = {label: np.zeros(len(levels)) for label in masks}

    for i, level in enumerate(levels):
        call_ex = calc_gamma_ex(level, df.StrikePrice, df.CallIV, df.daysTillExp,
                                0, 0, "call", df.CallOpenInt, contract_size)
        put_ex = calc_gamma_ex(level, df.StrikePrice, df.PutIV, df.daysTillExp,
                               0, 0, "put", df.PutOpenInt, contract_size)
        net = call_ex - put_ex
        for label, mask in masks.items():
            profiles[label][i] = net[mask].sum()

    # ---=== CHARM ET VANNA ===---
    # Même convention de signe que le GEX : dealers longs calls, shorts puts.
    # Calculés après le filtre d'échéance, donc sur le même périmètre que le reste.
    for nom, fonction in (("Charm", calc_charm_ex), ("Vanna", calc_vanna_ex)):
        c = fonction(spot_price, df.StrikePrice, df.CallIV, df.daysTillExp,
                     df.CallOpenInt, contract_size)
        p = fonction(spot_price, df.StrikePrice, df.PutIV, df.daysTillExp,
                     df.PutOpenInt, contract_size)
        df[f"Call{nom}"] = c
        df[f"Put{nom}"] = -p
        df[f"Total{nom}"] = c - p
    greeks_agg = df.groupby("StrikePrice")[["TotalCharm", "TotalVanna"]].sum()
    total_charm = df.TotalCharm.sum()
    total_vanna = df.TotalVanna.sum()

    profile = profiles["All Expiries"]
    zero_gamma = find_zero_gamma(levels, profile)
    if zero_gamma is None:
        print("Attention : pas de changement de signe du gamma dans la plage analysée "
              f"({from_strike:,.2f} - {to_strike:,.2f}) — élargis avec --range, "
              "ou allonge l'horizon avec --dte-max.")

    def fmt(x):
        return f"{x:,.{decimals}f}" if x is not None else "n/a"

    gscale, gunit = pick_scale([total_charm, total_vanna])
    print(f"Total GEX  : {total_gex / scale:,.2f} {unit} $ / mouvement de 1%")
    print(f"Zero Gamma : {fmt(zero_gamma)}")
    print(f"Call Wall  : {fmt(call_wall):>12} (gamma)   {fmt(call_wall_oi):>12} (open interest)")
    print(f"Put Wall   : {fmt(put_wall):>12} (gamma)   {fmt(put_wall_oi):>12} (open interest)")
    # Charm : delta que les dealers doivent racheter (négatif) ou revendre (positif)
    # pour chaque jour qui passe, à prix inchangé.
    print(f"Charm      : {total_charm / gscale:+,.2f} {gunit} $ de delta / jour de bourse")
    print(f"Vanna      : {total_vanna / gscale:+,.2f} {gunit} $ de delta / point de vol")

    # ---=== HISTORIQUE ===---
    # Une ligne par exécution : sans ça chaque analyse est un instantané, et les
    # séries n'existent nulle part. Le périmètre d'échéance est enregistré avec,
    # deux relevés du même jour sur des horizons différents n'étant pas comparables.
    if not args.no_history:
        import history
        history.record(args.history, timestamp=today_date, ticker=ticker,
                       dte_max="all" if args.dte_max is None else args.dte_max,
                       spot=spot_price, total_gex=total_gex, zero_gamma=zero_gamma,
                       call_wall=call_wall, put_wall=put_wall,
                       call_wall_oi=call_wall_oi, put_wall_oi=put_wall_oi,
                       charm=total_charm, vanna=total_vanna,
                       strikes=df.StrikePrice.nunique(),
                       expiries=df.ExpirationDate.nunique())

    os.makedirs(args.outdir, exist_ok=True)
    # L'horizon figure dans le titre : deux graphiques du même jour sur des périmètres
    # d'échéance différents donnent des niveaux différents, et rien ne les distinguerait.
    title_suffix = f"{ticker}, {today_date:%d %b %Y} ({horizon})"
    unit_label = f"Gamma Exposure ($ {unit} / mouvement de 1%)"

    # ---=== GRAPHIQUE 1 : GEX absolu par strike ===---
    fig, ax = plt.subplots(figsize=(12, 6))
    ax.grid(alpha=0.3)
    ax.bar(strikes, df_agg.TotalGamma / scale, width=width, linewidth=0.1,
           edgecolor="k", label="Gamma Exposure")
    ax.set_xlim([from_strike, to_strike])
    ax.set_title(f"Total Gamma : {total_gex / scale:,.2f} {unit} $ par mouvement de 1% — {title_suffix}",
                 fontweight="bold", fontsize=15)
    ax.set_xlabel("Strike", fontweight="bold")
    ax.set_ylabel(unit_label, fontweight="bold")
    annotate_levels(ax, spot_price, ticker, zero_gamma, call_wall, put_wall, decimals)
    ax.axhline(y=0, color="grey", lw=1)
    ax.legend()
    fig.tight_layout()
    fig.savefig(os.path.join(args.outdir, f"{ticker}_1_gamma_par_strike.png"), dpi=120)

    # ---=== GRAPHIQUE 2 : GEX calls vs puts ===---
    fig, ax = plt.subplots(figsize=(12, 6))
    ax.grid(alpha=0.3)
    ax.bar(strikes, df_agg.CallGEX / scale, width=width, linewidth=0.1,
           edgecolor="k", label="Call Gamma")
    ax.bar(strikes, df_agg.PutGEX / scale, width=width, linewidth=0.1,
           edgecolor="k", label="Put Gamma")
    ax.set_xlim([from_strike, to_strike])
    ax.set_title(f"Gamma calls vs puts — {title_suffix}", fontweight="bold", fontsize=15)
    ax.set_xlabel("Strike", fontweight="bold")
    ax.set_ylabel(unit_label, fontweight="bold")
    annotate_levels(ax, spot_price, ticker, zero_gamma, call_wall, put_wall, decimals)
    ax.axhline(y=0, color="grey", lw=1)
    ax.legend()
    fig.tight_layout()
    fig.savefig(os.path.join(args.outdir, f"{ticker}_2_calls_vs_puts.png"), dpi=120)

    # ---=== GRAPHIQUE 3 : profil de gamma et zero gamma level ===---
    fig, ax = plt.subplots(figsize=(12, 6))
    ax.grid(alpha=0.3)
    for label, values in profiles.items():
        if not np.all(values == 0):
            ax.plot(levels, values / scale, label=label)
    ax.set_title(f"Gamma Exposure Profile — {title_suffix}", fontweight="bold", fontsize=15)
    ax.set_xlabel("Prix du sous-jacent", fontweight="bold")
    ax.set_ylabel(unit_label, fontweight="bold")
    ax.set_xlim([from_strike, to_strike])
    annotate_levels(ax, spot_price, ticker, zero_gamma, decimals=decimals)
    ax.axhline(y=0, color="grey", lw=1)

    # Zones de régime : gamma négatif (déstabilisant) à gauche, positif à droite
    trans = ax.get_xaxis_transform()
    if zero_gamma is not None:
        ax.fill_between([from_strike, zero_gamma], 0, 1, facecolor="red", alpha=0.1,
                        transform=trans, label="Gamma négatif")
        ax.fill_between([zero_gamma, to_strike], 0, 1, facecolor="green", alpha=0.1,
                        transform=trans, label="Gamma positif")
        ax.annotate(f"Zero Gamma\n{zero_gamma:,.{decimals}f}", xy=(zero_gamma, 0.95), xycoords=trans,
                    ha="center", va="top", fontweight="bold", color="darkgreen",
                    bbox=dict(boxstyle="round,pad=0.3", fc="white", ec="darkgreen", alpha=0.85))
    ax.legend(loc="best")
    fig.tight_layout()
    fig.savefig(os.path.join(args.outdir, f"{ticker}_3_profil_zero_gamma.png"), dpi=120)

    # ---=== GRAPHIQUE 4 : charm et vanna par strike ===---
    fig, (ax_c, ax_v) = plt.subplots(2, 1, figsize=(12, 9), sharex=True)
    for ax, col, titre, ylab in (
            (ax_c, "TotalCharm", f"Charm : {total_charm/gscale:+,.2f} {gunit} $ de delta par jour de bourse",
             f"$ {gunit} de delta / jour de bourse"),
            (ax_v, "TotalVanna", f"Vanna : {total_vanna/gscale:+,.2f} {gunit} $ de delta par point de vol",
             f"$ {gunit} de delta / pt de vol")):
        ax.grid(alpha=0.3)
        ax.bar(greeks_agg.index.values, greeks_agg[col] / gscale, width=width,
               linewidth=0.1, edgecolor="k",
               color=["indianred" if v < 0 else "seagreen" for v in greeks_agg[col]])
        ax.set_xlim([from_strike, to_strike])
        ax.set_title(titre, fontweight="bold", fontsize=13)
        ax.set_ylabel(ylab, fontweight="bold")
        ax.axhline(y=0, color="grey", lw=1)
        ax.axvline(x=spot_price, color="r", lw=1.2, label=f"{ticker} Spot : {spot_price:,.{decimals}f}")
        if zero_gamma is not None:
            ax.axvline(x=zero_gamma, color="g", lw=1.4, ls="--",
                       label=f"Zero Gamma : {zero_gamma:,.{decimals}f}")
        ax.legend(fontsize=9)
    ax_v.set_xlabel("Strike", fontweight="bold")
    fig.suptitle(f"Charm & Vanna — {title_suffix}", fontweight="bold", fontsize=15)
    fig.tight_layout()
    fig.savefig(os.path.join(args.outdir, f"{ticker}_4_charm_vanna.png"), dpi=120)

    print(f"Graphiques enregistrés dans : {os.path.abspath(args.outdir)}")
    if not args.no_show:
        plt.show()


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as err:
        raise SystemExit(f"Erreur : {err}")
