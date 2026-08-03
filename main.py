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

pd.options.display.float_format = "{:,.4f}".format

CONTRACT_SIZE = 100
TRADING_DAYS = 262


def calc_gamma_ex(S, K, vol, T, r, q, opt_type, OI):
    """Gamma Black-Scholes ($ par mouvement de 1% du sous-jacent), vectorisé.

    S peut être un scalaire ou un vecteur de niveaux de spot ; K, vol, T et OI
    sont des vecteurs de même longueur (une entrée par contrat).
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

    return np.where(valid, OI * CONTRACT_SIZE * S * S * 0.01 * gamma, 0.0)


def is_third_friday(d):
    return d.weekday() == 4 and 15 <= d.day <= 21


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


def annotate_levels(ax, spot, ticker, zero_gamma=None, call_wall=None, put_wall=None):
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
                   label=f"{name} : {value:,.2f}{suffix}")

    draw(spot, "r", "-", f"{ticker} Spot", 1.2)
    draw(zero_gamma, "g", "--", "Zero Gamma", 1.6)
    draw(call_wall, "darkorange", ":", "Call Wall")
    draw(put_wall, "purple", ":", "Put Wall")


def main():
    parser = argparse.ArgumentParser(description="Gamma Exposure / Zero Gamma Level (données CBOE)")
    parser.add_argument("ticker", nargs="?", default="SPCX",
                        help="ticker du sous-jacent (SPCX, TSLA, _SPX...)")
    parser.add_argument("--csv", help="utiliser un export CSV CBOE au lieu de l'API JSON")
    parser.add_argument("--outdir", default="charts", help="dossier de sortie des graphiques")
    parser.add_argument("--no-show", action="store_true", help="enregistrer sans ouvrir les fenêtres")
    parser.add_argument("--range", type=float, default=0.2,
                        help="demi-plage de strikes autour du spot (0.2 = +/-20%%)")
    args = parser.parse_args()

    # ---=== CHARGEMENT DES DONNÉES ===---
    if args.csv:
        df, spot_price, today_date = cboe_data.load_from_csv(args.csv)
        ticker = args.ticker
    else:
        df, spot_price, today_date = cboe_data.fetch_chain(args.ticker)
        ticker = cboe_data.cboe_symbol(args.ticker).lstrip("_")

    from_strike = (1 - args.range) * spot_price
    to_strike = (1 + args.range) * spot_price
    print(f"{ticker} | spot {spot_price:,.2f} | {len(df)} strikes | {today_date:%Y-%m-%d %H:%M}")

    # ---=== GAMMA EXPOSURE PAR STRIKE ===---
    # GEX = gamma unitaire * OI * taille du contrat * spot, ramené à un mouvement de 1%
    df["CallGEX"] = df.CallGamma * df.CallOpenInt * CONTRACT_SIZE * spot_price ** 2 * 0.01
    df["PutGEX"] = df.PutGamma * df.PutOpenInt * CONTRACT_SIZE * spot_price ** 2 * 0.01 * -1
    df["TotalGamma"] = df.CallGEX + df.PutGEX

    df_agg = df.groupby("StrikePrice")[["CallGEX", "PutGEX", "TotalGamma"]].sum()
    in_range = df_agg[(df_agg.index >= from_strike) & (df_agg.index <= to_strike)]
    strikes = df_agg.index.values

    # Murs de gamma : strikes concentrant le plus de gamma call (résistance) / put (support).
    # Calculés sur une bande large fixe (+/-50%) pour ne pas dépendre du zoom d'affichage.
    wall_band = df_agg[(df_agg.index >= 0.5 * spot_price) & (df_agg.index <= 1.5 * spot_price)]
    call_wall = wall_band.CallGEX.idxmax() if len(wall_band) else None
    put_wall = wall_band.PutGEX.idxmin() if len(wall_band) else None

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
        "Ex-Next Monthly Expiry": (df.ExpirationDate != next_monthly_exp).values,
    }
    profiles = {label: np.zeros(len(levels)) for label in masks}

    for i, level in enumerate(levels):
        call_ex = calc_gamma_ex(level, df.StrikePrice, df.CallIV, df.daysTillExp,
                                0, 0, "call", df.CallOpenInt)
        put_ex = calc_gamma_ex(level, df.StrikePrice, df.PutIV, df.daysTillExp,
                               0, 0, "put", df.PutOpenInt)
        net = call_ex - put_ex
        for label, mask in masks.items():
            profiles[label][i] = net[mask].sum()

    profile = profiles["All Expiries"]
    zero_gamma = find_zero_gamma(levels, profile)
    if zero_gamma is None:
        print("Attention : pas de changement de signe du gamma dans la plage analysée "
              f"({from_strike:,.2f} - {to_strike:,.2f}) — élargis avec --range.")

    print(f"Total GEX  : {total_gex / scale:,.2f} {unit} $ / mouvement de 1%")
    print(f"Zero Gamma : {zero_gamma:,.2f}" if zero_gamma else "Zero Gamma : introuvable")
    print(f"Call Wall  : {call_wall:,.2f}" if call_wall else "Call Wall  : n/a")
    print(f"Put Wall   : {put_wall:,.2f}" if put_wall else "Put Wall   : n/a")

    os.makedirs(args.outdir, exist_ok=True)
    title_suffix = f"{ticker}, {today_date:%d %b %Y}"
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
    annotate_levels(ax, spot_price, ticker, zero_gamma, call_wall, put_wall)
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
    annotate_levels(ax, spot_price, ticker, zero_gamma, call_wall, put_wall)
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
    annotate_levels(ax, spot_price, ticker, zero_gamma)
    ax.axhline(y=0, color="grey", lw=1)

    # Zones de régime : gamma négatif (déstabilisant) à gauche, positif à droite
    trans = ax.get_xaxis_transform()
    if zero_gamma is not None:
        ax.fill_between([from_strike, zero_gamma], 0, 1, facecolor="red", alpha=0.1,
                        transform=trans, label="Gamma négatif")
        ax.fill_between([zero_gamma, to_strike], 0, 1, facecolor="green", alpha=0.1,
                        transform=trans, label="Gamma positif")
        ax.annotate(f"Zero Gamma\n{zero_gamma:,.2f}", xy=(zero_gamma, 0.95), xycoords=trans,
                    ha="center", va="top", fontweight="bold", color="darkgreen",
                    bbox=dict(boxstyle="round,pad=0.3", fc="white", ec="darkgreen", alpha=0.85))
    ax.legend(loc="best")
    fig.tight_layout()
    fig.savefig(os.path.join(args.outdir, f"{ticker}_3_profil_zero_gamma.png"), dpi=120)

    print(f"Graphiques enregistrés dans : {os.path.abspath(args.outdir)}")
    if not args.no_show:
        plt.show()


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as err:
        raise SystemExit(f"Erreur : {err}")
