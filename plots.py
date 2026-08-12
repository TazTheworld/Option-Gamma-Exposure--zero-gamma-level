"""Les quatre graphiques, à partir d'une Analyse.

Séparés du calcul : rien ici ne produit un chiffre, tout y est mis en forme.
matplotlib bascule automatiquement sur le backend non interactif quand aucune
fenêtre n'est possible (CI, session sans affichage).
"""

import os

import matplotlib
import numpy as np

if not os.environ.get("DISPLAY") and os.name != "nt":
    matplotlib.use("Agg")

import matplotlib.pyplot as plt  # noqa: E402

from analysis import pick_scale  # noqa: E402


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


def _largeur_barres(par_strike, from_strike, to_strike):
    """Largeur adaptée à l'espacement réel des strikes de la fenêtre affichée."""
    dedans = par_strike[(par_strike.index >= from_strike) & (par_strike.index <= to_strike)]
    espacement = np.median(np.diff(dedans.index.values)) if len(dedans) > 1 else 1.0
    return espacement * 0.8


def tracer(analyse, outdir="charts", dpi=120):
    """Écrit les quatre graphiques et renvoie la liste des chemins."""
    os.makedirs(outdir, exist_ok=True)
    a = analyse
    dedans = a.par_strike[(a.par_strike.index >= a.from_strike)
                          & (a.par_strike.index <= a.to_strike)]
    scale, unit = pick_scale(dedans.TotalGamma.values if len(dedans)
                             else a.par_strike.TotalGamma.values)
    gscale, gunit = pick_scale([a.total_charm, a.total_vanna])
    width = _largeur_barres(a.par_strike, a.from_strike, a.to_strike)
    strikes = a.par_strike.index.values
    dec = a.decimals

    # L'horizon figure dans le titre : deux graphiques du même jour sur des périmètres
    # d'échéance différents donnent des niveaux différents, et rien ne les distinguerait.
    suffixe = (f"{a.ticker}, {a.quote_date:%d %b %Y} "
               f"({a.horizon}, gamma {a.source_gamma}, T {a.time_convention})")
    ylabel = f"Gamma Exposure ($ {unit} / mouvement de 1%)"
    chemins = []

    def enregistrer(fig, nom):
        chemin = os.path.join(outdir, f"{a.ticker}_{nom}.png")
        fig.tight_layout()
        fig.savefig(chemin, dpi=dpi)
        chemins.append(chemin)

    # ---=== GRAPHIQUE 1 : GEX absolu par strike ===---
    fig, ax = plt.subplots(figsize=(12, 6))
    ax.grid(alpha=0.3)
    ax.bar(strikes, a.par_strike.TotalGamma / scale, width=width, linewidth=0.1,
           edgecolor="k", label="Gamma Exposure")
    ax.set_xlim([a.from_strike, a.to_strike])
    ax.set_title(f"Total Gamma : {a.total_gex / scale:,.2f} {unit} $ par mouvement de 1% "
                 f"— {suffixe}", fontweight="bold", fontsize=15)
    ax.set_xlabel("Strike", fontweight="bold")
    ax.set_ylabel(ylabel, fontweight="bold")
    annotate_levels(ax, a.spot, a.ticker, a.zero_gamma, a.call_wall, a.put_wall, dec)
    ax.axhline(y=0, color="grey", lw=1)
    ax.legend()
    enregistrer(fig, "1_gamma_par_strike")

    # ---=== GRAPHIQUE 2 : GEX calls vs puts ===---
    fig, ax = plt.subplots(figsize=(12, 6))
    ax.grid(alpha=0.3)
    ax.bar(strikes, a.par_strike.CallGEX / scale, width=width, linewidth=0.1,
           edgecolor="k", label="Call Gamma")
    ax.bar(strikes, a.par_strike.PutGEX / scale, width=width, linewidth=0.1,
           edgecolor="k", label="Put Gamma")
    ax.set_xlim([a.from_strike, a.to_strike])
    ax.set_title(f"Gamma calls vs puts — {suffixe}", fontweight="bold", fontsize=15)
    ax.set_xlabel("Strike", fontweight="bold")
    ax.set_ylabel(ylabel, fontweight="bold")
    annotate_levels(ax, a.spot, a.ticker, a.zero_gamma, a.call_wall, a.put_wall, dec)
    ax.axhline(y=0, color="grey", lw=1)
    ax.legend()
    enregistrer(fig, "2_calls_vs_puts")

    # ---=== GRAPHIQUE 3 : profil de gamma et zero gamma level ===---
    fig, ax = plt.subplots(figsize=(12, 6))
    ax.grid(alpha=0.3)
    for label, values in a.profiles.items():
        if not np.all(values == 0):
            ax.plot(a.levels, values / scale, label=label)
    ax.set_title(f"Gamma Exposure Profile — {suffixe}", fontweight="bold", fontsize=15)
    ax.set_xlabel("Prix du sous-jacent", fontweight="bold")
    ax.set_ylabel(ylabel, fontweight="bold")
    ax.set_xlim([a.from_strike, a.to_strike])
    annotate_levels(ax, a.spot, a.ticker, a.zero_gamma, decimals=dec)
    ax.axhline(y=0, color="grey", lw=1)

    # Zones de régime : gamma négatif (déstabilisant) à gauche, positif à droite
    trans = ax.get_xaxis_transform()
    if a.zero_gamma is not None:
        ax.fill_between([a.from_strike, a.zero_gamma], 0, 1, facecolor="red", alpha=0.1,
                        transform=trans, label="Gamma négatif")
        ax.fill_between([a.zero_gamma, a.to_strike], 0, 1, facecolor="green", alpha=0.1,
                        transform=trans, label="Gamma positif")
        ax.annotate(f"Zero Gamma\n{a.zero_gamma:,.{dec}f}", xy=(a.zero_gamma, 0.95),
                    xycoords=trans, ha="center", va="top", fontweight="bold",
                    color="darkgreen",
                    bbox=dict(boxstyle="round,pad=0.3", fc="white", ec="darkgreen", alpha=0.85))
    # Les croisements secondaires sont tracés en pointillés : le régime n'est pas
    # une simple bascule quand le profil repasse par zéro plusieurs fois.
    for autre in a.croisements:
        if a.zero_gamma is not None and abs(autre - a.zero_gamma) < 1e-9:
            continue
        ax.axvline(x=autre, color="g", lw=0.8, ls=":", alpha=0.6,
                   label=f"autre croisement : {autre:,.{dec}f}")
    ax.legend(loc="best")
    enregistrer(fig, "3_profil_zero_gamma")

    # ---=== GRAPHIQUE 4 : charm et vanna par strike ===---
    fig, (ax_c, ax_v) = plt.subplots(2, 1, figsize=(12, 9), sharex=True)
    for ax, col, titre, ylab in (
            (ax_c, "TotalCharm",
             f"Charm : {a.total_charm / gscale:+,.2f} {gunit} $ de delta par jour",
             f"$ {gunit} de delta / jour"),
            (ax_v, "TotalVanna",
             f"Vanna : {a.total_vanna / gscale:+,.2f} {gunit} $ de delta par point de vol",
             f"$ {gunit} de delta / pt de vol")):
        ax.grid(alpha=0.3)
        ax.bar(a.par_strike.index.values, a.par_strike[col] / gscale, width=width,
               linewidth=0.1, edgecolor="k",
               color=["indianred" if v < 0 else "seagreen" for v in a.par_strike[col]])
        ax.set_xlim([a.from_strike, a.to_strike])
        ax.set_title(titre, fontweight="bold", fontsize=13)
        ax.set_ylabel(ylab, fontweight="bold")
        ax.axhline(y=0, color="grey", lw=1)
        ax.axvline(x=a.spot, color="r", lw=1.2, label=f"{a.ticker} Spot : {a.spot:,.{dec}f}")
        if a.zero_gamma is not None:
            ax.axvline(x=a.zero_gamma, color="g", lw=1.4, ls="--",
                       label=f"Zero Gamma : {a.zero_gamma:,.{dec}f}")
        ax.legend(fontsize=9)
    ax_v.set_xlabel("Strike", fontweight="bold")
    fig.suptitle(f"Charm & Vanna — {suffixe}", fontweight="bold", fontsize=15)
    enregistrer(fig, "4_charm_vanna")

    return chemins


def afficher():
    plt.show()


def fermer():
    plt.close("all")
