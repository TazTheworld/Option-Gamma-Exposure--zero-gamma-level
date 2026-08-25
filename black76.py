"""Black-76 : les options sur futures, et le multiplicateur de chaque contrat.

Black-76 n'est pas Black-Scholes en habit neuf. Le sous-jacent est un future : il
ne porte ni dividende ni coût de portage, et la prime entière s'actualise au taux
sans risque. `greeks.py` garde donc les actions, ce module les futures — les
fondre en une seule fonction à deux régimes ferait exactement l'erreur qu'on ne
voit pas passer, puisqu'un gamma calculé dans le mauvais régime reste un nombre
plausible.

Ce code vivait dans `cme_data.py`, qui était à la fois la source des exports de
règlement CME et le seul endroit où Black-76 était écrit. La source est partie ;
la formule, elle, sert toujours à `ib_data.py`, qui recalcule le gamma là où IB
ne le publie pas.
"""

import numpy as np
from scipy.optimize import brentq
from scipy.stats import norm

# Multiplicateur de chaque contrat, en unités de sous-jacent. NQ est le E-mini,
# à x20 — le contrat *full size* ND, à x100, n'existe plus, et MNQ (Micro) vaut
# x2. Se tromper de ligne ne produit aucune erreur visible : un GEX cinq fois
# trop grand reste un nombre plausible, d'où taille_contrat() qui refuse de
# deviner plutôt que de servir un défaut.
CONTRACT_SIZES = {"6E": 125_000, "6B": 62_500, "6J": 12_500_000, "6A": 100_000,
                  "ES": 50, "NQ": 20, "CL": 1_000, "GC": 100}


def taille_contrat(ticker):
    """Le multiplicateur du produit, ou une erreur qui dit lesquels sont connus.

    Un défaut silencieux serait le pire des deux mondes : le calcul aboutirait, et
    le chiffre serait faux d'un facteur entier sans que rien ne le signale.
    """
    propre = str(ticker).upper()
    if propre not in CONTRACT_SIZES:
        connus = ", ".join(sorted(CONTRACT_SIZES))
        raise ValueError(
            f"Multiplicateur inconnu pour {propre!r}. Produits connus : {connus}. "
            f"Passe-le explicitement avec --contract-size."
        )
    return CONTRACT_SIZES[propre]


def _d1(F, K, vol, T):
    return (np.log(F / K) + 0.5 * vol ** 2 * T) / (vol * np.sqrt(T))


def black76_price(F, K, vol, T, r, opt_type):
    if T <= 0 or vol <= 0:
        return max(F - K, 0.0) if opt_type == "C" else max(K - F, 0.0)
    d1 = _d1(F, K, vol, T)
    d2 = d1 - vol * np.sqrt(T)
    if opt_type == "C":
        return np.exp(-r * T) * (F * norm.cdf(d1) - K * norm.cdf(d2))
    return np.exp(-r * T) * (K * norm.cdf(-d2) - F * norm.cdf(-d1))


def black76_gamma(F, K, vol, T, r=0.0):
    """Gamma d'une option sur future. À r = 0, identique au gamma Black-Scholes."""
    F, K, vol, T = (np.asarray(x, dtype=float) for x in (F, K, vol, T))
    valid = (T > 0) & (vol > 0) & (K > 0) & (F > 0)
    vol_s, T_s, K_s, F_s = (np.where(valid, x, 1.0) for x in (vol, T, K, F))
    d1 = _d1(F_s, K_s, vol_s, T_s)
    gamma = np.exp(-r * T_s) * norm.pdf(d1) / (F_s * vol_s * np.sqrt(T_s))
    return np.where(valid, gamma, 0.0)


def implied_vol(price, F, K, T, r, opt_type):
    """Inverse la vol depuis le prix de règlement (utilisé si l'IV manque)."""
    intrinsic = max(F - K, 0.0) if opt_type == "C" else max(K - F, 0.0)
    if not np.isfinite(price) or price <= intrinsic * np.exp(-r * T) or T <= 0:
        return np.nan
    try:
        return brentq(lambda v: black76_price(F, K, v, T, r, opt_type) - price,
                      1e-6, 5.0, xtol=1e-8, maxiter=200)
    except (ValueError, RuntimeError):
        return np.nan


def infer_futures_price(df):
    """Déduit le prix du future par parité call-put au strike le plus ATM.

    Black-76 à r = 0 : C - P = F - K, donc F = K + (C - P) là où |C - P| est
    minimal (le strike le plus proche de la monnaie). IB sert le prix dans
    `undPrice`, donc ceci ne sert plus que de filet — pour un relevé qui n'en
    aurait pas.
    """
    ok = df.dropna(subset=["CallSettle", "PutSettle", "StrikePrice"])
    if ok.empty:
        return None
    diff = (ok.CallSettle - ok.PutSettle).abs()
    row = ok.loc[diff.idxmin()]
    return float(row.StrikePrice + row.CallSettle - row.PutSettle)
