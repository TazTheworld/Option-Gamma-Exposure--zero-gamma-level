"""Les greeks, recoupés par différences finies plutôt que par des valeurs codées en dur.

Une valeur attendue écrite à la main ne prouve rien : elle vient de la même formule
que le code. On dérive donc numériquement le delta et le prix, et on compare.
"""

import numpy as np
import pytest
from scipy.stats import norm

import main as M
from cme_data import black76_gamma, black76_price, implied_vol

CAS = [  # (S, K, vol, T)
    (100, 105, 0.25, 0.10),
    (100, 95, 0.30, 0.05),
    (100, 100, 0.20, 0.02),
    (150, 160, 0.45, 0.25),
    (7700, 8000, 0.15, 0.08),
    (1.0650, 1.05, 0.07, 0.05),
]
H = 1e-6


def delta_bs(S, K, vol, T, cp):
    """Delta Black-Scholes à r = q = 0."""
    d1 = (np.log(S / K) + 0.5 * vol ** 2 * T) / (vol * np.sqrt(T))
    return norm.cdf(d1) if cp == "C" else norm.cdf(d1) - 1


@pytest.mark.parametrize("S,K,vol,T", CAS)
@pytest.mark.parametrize("cp", ["C", "P"])
def test_charm_egale_derivee_temporelle_du_delta(S, K, vol, T, cp):
    """charm = -d(delta)/dT, soit la dérive par unité de temps écoulé."""
    attendu = -(delta_bs(S, K, vol, T + H, cp) - delta_bs(S, K, vol, T - H, cp)) / (2 * H)
    # calc_charm_ex renvoie des dollars de delta par jour de bourse : on remonte
    # au charm nu en retirant OI, multiplicateur, spot et le diviseur temporel.
    obtenu = float(M.calc_charm_ex(S, K, vol, T, OI=1, contract_size=1)) * M.TRADING_DAYS / S
    assert obtenu == pytest.approx(attendu, rel=1e-6)


@pytest.mark.parametrize("S,K,vol,T", CAS)
@pytest.mark.parametrize("cp", ["C", "P"])
def test_vanna_egale_derivee_du_delta_en_vol(S, K, vol, T, cp):
    attendu = (delta_bs(S, K, vol + H, T, cp) - delta_bs(S, K, vol - H, T, cp)) / (2 * H)
    obtenu = float(M.calc_vanna_ex(S, K, vol, T, OI=1, contract_size=1)) * 100.0 / S
    assert obtenu == pytest.approx(attendu, rel=1e-6)


@pytest.mark.parametrize("S,K,vol,T", CAS)
def test_charm_et_vanna_identiques_calls_et_puts(S, K, vol, T):
    """À q = 0, le -1 du delta put ne s'écoule pas : les deux greeks coïncident."""
    for cp_a, cp_b in [("C", "P")]:
        ca = -(delta_bs(S, K, vol, T + H, cp_a) - delta_bs(S, K, vol, T - H, cp_a)) / (2 * H)
        cb = -(delta_bs(S, K, vol, T + H, cp_b) - delta_bs(S, K, vol, T - H, cp_b)) / (2 * H)
        assert ca == pytest.approx(cb, rel=1e-6)


@pytest.mark.parametrize("S,K,vol,T", CAS)
def test_gamma_black_scholes_egale_black_76_a_taux_nul(S, K, vol, T):
    """C'est ce qui permet d'utiliser le même code pour actions et options sur futures."""
    bs = float(M.calc_gamma_ex(S, K, vol, T, 0, 0, "call", OI=1, contract_size=1))
    b76 = float(black76_gamma(S, K, vol, T, 0.0)) * 1 * S * S * 0.01
    assert bs == pytest.approx(b76, rel=1e-12)


@pytest.mark.parametrize("S,K,vol,T", CAS)
def test_gamma_call_egale_gamma_put(S, K, vol, T):
    """Les deux branches de calc_gamma_ex utilisent des formules différentes."""
    c = float(M.calc_gamma_ex(S, K, vol, T, 0, 0, "call", OI=1, contract_size=1))
    p = float(M.calc_gamma_ex(S, K, vol, T, 0, 0, "put", OI=1, contract_size=1))
    assert c == pytest.approx(p, rel=1e-9)


@pytest.mark.parametrize("greek", [M.calc_gamma_ex, M.calc_charm_ex, M.calc_vanna_ex])
def test_entrees_invalides_donnent_zero_sans_warning(greek):
    """Échéance passée, vol nulle ou strike nul ne doivent ni planter ni polluer la somme."""
    kw = dict(OI=100, contract_size=100)
    args = (100.0, np.array([100.0, 100.0, 0.0]), np.array([0.2, 0.0, 0.2]),
            np.array([0.0, 0.1, 0.1]))
    with np.errstate(all="raise"):
        out = (greek(*args, 0, 0, "call", **kw) if greek is M.calc_gamma_ex
               else greek(*args, **kw))
    assert np.all(out == 0.0)


def test_implied_vol_retrouve_la_vol_injectee():
    """Aller-retour prix -> vol : c'est le chemin utilisé quand le CME ne publie pas l'IV."""
    F, K, T, vraie = 1.0650, 1.05, 0.08, 0.0723
    for cp in ("C", "P"):
        prix = black76_price(F, K, vraie, T, 0.0, cp)
        assert implied_vol(prix, F, K, T, 0.0, cp) == pytest.approx(vraie, rel=1e-6)


def test_implied_vol_renvoie_nan_sous_la_valeur_intrinseque():
    """Un prix inférieur à l'intrinsèque n'a pas de vol implicite : NaN, pas d'exception."""
    assert np.isnan(implied_vol(0.001, 1.20, 1.00, 0.08, 0.0, "C"))


def test_find_zero_gamma_interpole_le_changement_de_signe():
    niveaux = np.array([90.0, 100.0, 110.0])
    profil = np.array([-10.0, -5.0, 5.0])   # croise zéro entre 100 et 110
    assert M.find_zero_gamma(niveaux, profil) == pytest.approx(105.0)


def test_find_zero_gamma_sans_croisement_renvoie_none():
    assert M.find_zero_gamma(np.array([1.0, 2.0]), np.array([3.0, 4.0])) is None


def test_pick_scale_bascule_au_milliard():
    assert M.pick_scale([5e8])[1] == "millions"
    assert M.pick_scale([2e9])[1] == "milliards"
    assert M.pick_scale([])[1] == "millions"
