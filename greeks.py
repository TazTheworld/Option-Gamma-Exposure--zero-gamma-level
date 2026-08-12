"""Greeks Black-Scholes vectorisés, à r = q = 0.

Isolés de tout le reste : pas d'accès réseau, pas de pandas, pas d'affichage.
C'est le seul endroit où une formule est écrite, et il est intégralement couvert
par différences finies dans tests/test_greeks.py.

Toutes les fonctions acceptent un spot scalaire OU un vecteur de niveaux. Avec un
spot de forme (L, 1) et des contrats de forme (N,), le résultat est de forme
(L, N) : c'est ce qui permet de calculer le profil de gamma sur toute une grille
de niveaux d'un coup, sans boucle Python.

À r = q = 0 ces formules sont identiquement égales à leurs équivalents Black-76,
donc elles s'appliquent telles quelles aux options sur futures (6E, ES...) :
seule la taille du contrat change.
"""

import numpy as np
from scipy.stats import norm

CONTRACT_SIZE = 100          # actions et ETF US ; 125 000 pour le 6E (Euro FX)
TRADING_DAYS = 262


def calc_gamma_ex(S, K, vol, T, r, q, opt_type, OI, contract_size=CONTRACT_SIZE):
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

    return np.where(valid, OI * contract_size * S * S * 0.01 * gamma, 0.0)


def _d1_d2(S, K, vol, T):
    """d1 et d2 de Black-Scholes à r = q = 0, avec neutralisation des entrées invalides."""
    S = np.asarray(S, dtype=float)
    K, vol, T = (np.asarray(x, dtype=float) for x in (K, vol, T))
    valid = (T > 0) & (vol > 0) & (K > 0)
    vol_s, T_s, K_s = (np.where(valid, x, 1.0) for x in (vol, T, K))
    d1 = (np.log(S / K_s) + 0.5 * vol_s ** 2 * T_s) / (vol_s * np.sqrt(T_s))
    return d1, d1 - vol_s * np.sqrt(T_s), valid, vol_s, T_s


def calc_charm_ex(S, K, vol, T, OI, contract_size=CONTRACT_SIZE, jours_par_an=TRADING_DAYS):
    """Charm exposure : dollars de delta gagnés par jour, à prix constant.

    charm = phi(d1) * d2 / (2T). À q = 0 il est identique pour calls et puts
    (le -1 du delta put ne s'écoule pas).

    La dérivée est par unité de T : `jours_par_an` doit donc valoir le même
    diviseur que celui ayant servi à calculer T — 365 en convention "heures",
    TRADING_DAYS en convention "bourse". analysis.time_to_expiry() renvoie les
    deux ensemble pour éviter de les désaccorder. Vérifié par différence finie
    sur le delta dollar du book complet.

    Interprétation : un charm exposure positif signifie que le delta du book
    dealer grossit avec le temps, donc qu'il doit vendre pour rester neutre.
    C'est le flux de couverture des derniers jours avant échéance, celui que le
    gamma seul ne montre pas.
    """
    d1, d2, valid, _, T_s = _d1_d2(S, K, vol, T)
    charm = norm.pdf(d1) * d2 / (2 * T_s)
    OI = np.asarray(OI, dtype=float)
    return np.where(valid, OI * contract_size * np.asarray(S, float) * charm / jours_par_an, 0.0)


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
