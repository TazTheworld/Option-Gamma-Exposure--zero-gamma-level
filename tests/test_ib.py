"""Les fonctions pures du collecteur Interactive Brokers.

Comme pour databento, les jeux d'essai sont fabriqués depuis des paramètres
connus : on vérifie qu'on retrouve la vérité terrain, pas qu'une sortie est figée.

Aucun test ne touche au réseau, et rien ici n'importe ib_async — la dépendance de
courtier est un extra, le cœur du projet tient à cinq paquets.
"""

import numpy as np
import pandas as pd
import pytest

import ib_data
from cboe_data import COLUMNS

QUOTE = pd.Timestamp("2026-08-25")
PRIX = 25_000.0
STRIKES = np.arange(20_000.0, 30_025.0, 25.0)
ECHEANCES = ["20260826", "20260828", "20260904", "20260918", "20261120"]


# ---=== perimetre ===---

def test_perimetre_coupe_les_strikes_hors_plage():
    """±2 % autour de 25 000, c'est 24 500 à 25 500 et rien d'autre."""
    p = ib_data.perimetre(STRIKES, ["20260904"], PRIX, plage=0.02, quote_date=QUOTE)
    assert p.StrikePrice.min() == pytest.approx(24_500.0)
    assert p.StrikePrice.max() == pytest.approx(25_500.0)


def test_perimetre_coupe_les_echeances_hors_horizon():
    """dte_max=30 écarte le 20 novembre, dte_min=2 écarte le lendemain."""
    p = ib_data.perimetre(STRIKES, ECHEANCES, PRIX, plage=0.01,
                          dte_max=30, dte_min=2, quote_date=QUOTE)
    gardees = sorted(pd.Timestamp(d).strftime("%Y%m%d") for d in p.ExpirationDate.unique())
    assert gardees == ["20260828", "20260904", "20260918"]


def test_perimetre_produit_les_deux_cotes():
    """Un contrat par (échéance, strike, sens) : le GEX a besoin des deux jambes."""
    p = ib_data.perimetre([25_000.0], ["20260904"], PRIX, quote_date=QUOTE)
    assert sorted(p.right) == ["C", "P"]
    assert len(p) == 2


def test_perimetre_compte_les_contrats_a_demander():
    """C'est ce nombre qui décide du temps de balayage : il doit être prévisible."""
    p = ib_data.perimetre(STRIKES, ECHEANCES, PRIX, plage=0.025,
                          dte_max=30, dte_min=0, quote_date=QUOTE)
    n_strikes = p.StrikePrice.nunique()
    n_echeances = p.ExpirationDate.nunique()
    assert len(p) == n_strikes * n_echeances * 2
    assert n_strikes == 51            # 25 de part et d'autre, plus la monnaie


def test_perimetre_dte_max_none_garde_tout():
    """Le 'all' de main.py arrive ici en None : aucune échéance ne doit tomber."""
    p = ib_data.perimetre([25_000.0], ECHEANCES, PRIX, dte_max=None, quote_date=QUOTE)
    assert p.ExpirationDate.nunique() == len(ECHEANCES)


def test_perimetre_vide_rend_une_trame_typee_pas_une_erreur():
    """Une grille sans strike dans la plage n'est pas une panne : le CME ne liste
    que vingt-cinq strikes autour du règlement sur les échéances courtes."""
    p = ib_data.perimetre(STRIKES, ECHEANCES, prix=1.0, plage=0.02, quote_date=QUOTE)
    assert p.empty
    assert list(p.columns) == ["ExpirationDate", "StrikePrice", "right"]


def test_perimetre_dedoublonne_et_trie():
    """reqSecDefOptParams rend des ensembles : l'unicité et l'ordre sont à nous.

    L'ordre compte pour de vrai : c'est celui dans lequel les lots partiront, et
    deux appels qui rendraient deux ordres différents feraient re-souscrire les
    mêmes contrats à chaque recyclage.
    """
    p = ib_data.perimetre([25_000.0, 25_000.0, 24_975.0], ["20260904", "20260904"],
                          PRIX, plage=0.01, quote_date=QUOTE)
    assert p.StrikePrice.nunique() == 2
    assert p.ExpirationDate.nunique() == 1
    assert len(p) == 4                                  # 2 strikes x 2 sens
    assert p.StrikePrice.is_monotonic_increasing
    assert list(p.right) == ["C", "P", "C", "P"]


# ---=== build_chain ===---

VOL_VRAI = 0.18
EXP = pd.Timestamp("2026-09-04 14:00")
T_VRAI = (EXP - QUOTE).total_seconds() / (365.25 * 24 * 3600)


def _trames_ib(avec_gamma=True, iv_en_pourcent=False, oi=500.0):
    """Imite ce que la couche réseau produira : définitions + valeurs par conId."""
    from cme_data import black76_gamma
    strikes = np.arange(24_500.0, 25_525.0, 25.0)
    defs, ticks, cid = [], [], 100_000
    for k in strikes:
        for r in ("C", "P"):
            defs.append({"conId": cid, "StrikePrice": k,
                         "ExpirationDate": EXP, "right": r})
            gamma = float(black76_gamma(PRIX, k, VOL_VRAI, T_VRAI)) if avec_gamma else np.nan
            ticks.append({
                "conId": cid,
                "OpenInt": oi,
                "IV": VOL_VRAI * 100 if iv_en_pourcent else VOL_VRAI,
                "Gamma": gamma,
                "Delta": np.nan, "Vega": np.nan, "Theta": np.nan,
                "Settle": np.nan,
            })
            cid += 1
    # bruit : un contrat sans valeur reçue, comme un strike illiquide qui ne
    # répond jamais à la souscription
    defs.append({"conId": 999_999, "StrikePrice": 25_000.0,
                 "ExpirationDate": EXP, "right": "C"})
    return pd.DataFrame(defs), pd.DataFrame(ticks)


def test_build_chain_rend_le_format_pivot():
    """Tout l'aval (analysis, plots, snapshots) attend COLUMNS et rien d'autre."""
    chaine, prix, date_val = ib_data.build_chain(
        *_trames_ib(), futures_price=PRIX, quote_date=QUOTE)
    for colonne in COLUMNS:
        assert colonne in chaine.columns
    assert prix == pytest.approx(PRIX)
    assert pd.Timestamp(date_val) == QUOTE


def test_build_chain_une_ligne_par_strike():
    """Format large : calls et puts d'un même strike sur la même ligne."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert len(chaine) == 41                       # 24 500 à 25 500 au pas de 25
    assert chaine.StrikePrice.is_unique


def test_build_chain_reporte_l_open_interest():
    """Sans OI il n'y a pas de GEX : c'est l'entrée principale d'expositions()."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(oi=500.0), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert chaine.CallOpenInt.sum() == pytest.approx(500.0 * 41)
    assert chaine.PutOpenInt.sum() == pytest.approx(500.0 * 41)


def test_build_chain_normalise_l_iv_en_pourcentage():
    """IB publie parfois l'IV en pourcent : 18 n'est pas 1 800 % de volatilité."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(iv_en_pourcent=True),
                                       futures_price=PRIX, quote_date=QUOTE)
    assert chaine.CallIV.mean() == pytest.approx(VOL_VRAI, abs=1e-6)


def test_build_chain_recalcule_le_gamma_absent_en_black76():
    """Si IB ne publie pas le gamma, Black-76 le retrouve depuis l'IV."""
    sans, _, _ = ib_data.build_chain(*_trames_ib(avec_gamma=False),
                                     futures_price=PRIX, quote_date=QUOTE)
    avec, _, _ = ib_data.build_chain(*_trames_ib(avec_gamma=True),
                                     futures_price=PRIX, quote_date=QUOTE)
    assert (sans.CallGamma > 0).any()
    assert sans.CallGamma.to_numpy() == pytest.approx(avec.CallGamma.to_numpy(), rel=1e-6)


def test_build_chain_garde_le_gamma_publie_quand_il_existe():
    """--gamma-source published doit avoir de quoi se nourrir."""
    from cme_data import black76_gamma
    chaine, _, _ = ib_data.build_chain(*_trames_ib(avec_gamma=True),
                                       futures_price=PRIX, quote_date=QUOTE)
    atm = chaine.loc[(chaine.StrikePrice - PRIX).abs().idxmin()]
    attendu = float(black76_gamma(PRIX, atm.StrikePrice, VOL_VRAI, T_VRAI))
    assert atm.CallGamma == pytest.approx(attendu, rel=1e-6)


def test_build_chain_ignore_les_contrats_sans_valeur():
    """Un strike illiquide qui ne répond jamais ne doit pas casser l'assemblage."""
    defs, ticks = _trames_ib()
    assert 999_999 in set(defs.conId)          # présent dans les définitions
    assert 999_999 not in set(ticks.conId)     # absent des valeurs reçues
    chaine, _, _ = ib_data.build_chain(defs, ticks, futures_price=PRIX,
                                       quote_date=QUOTE)
    assert len(chaine) == 41                   # il n'ajoute pas de ligne fantôme
    atm = chaine[chaine.StrikePrice == 25_000.0].iloc[0]
    assert atm.CallOpenInt == pytest.approx(500.0)   # la vraie valeur survit


def test_build_chain_definitions_vides_leve_une_erreur():
    """Échouer bruyamment : une chaîne vide silencieuse donnerait un GEX de zéro."""
    with pytest.raises(ValueError):
        ib_data.build_chain(pd.DataFrame(columns=["conId", "StrikePrice",
                                                  "ExpirationDate", "right"]),
                            pd.DataFrame(columns=["conId"]),
                            futures_price=PRIX, quote_date=QUOTE)


def test_build_chain_deduit_le_prix_du_future_par_parite():
    """Sans prix fourni, la parité call-put le retrouve — comme pour le CME."""
    from cme_data import black76_price
    defs, ticks = _trames_ib()
    prix_par_conid = {}
    for _, d in defs.iterrows():
        if d.conId == 999_999:
            continue
        prix_par_conid[d.conId] = black76_price(PRIX, d.StrikePrice, VOL_VRAI,
                                                T_VRAI, 0.0, d.right)
    ticks = ticks.copy()
    ticks["Settle"] = ticks.conId.map(prix_par_conid)

    _, prix, _ = ib_data.build_chain(defs, ticks, futures_price=None,
                                     quote_date=QUOTE)
    assert prix == pytest.approx(PRIX, rel=1e-4)


def test_build_chain_alimente_analyser_sans_retouche():
    """Le contrat de bout en bout : la sortie entre telle quelle dans analysis."""
    import analysis
    chaine, prix, date_val = ib_data.build_chain(*_trames_ib(),
                                                 futures_price=PRIX,
                                                 quote_date=QUOTE)
    a = analysis.analyser(chaine, spot=prix, quote_date=date_val,
                          ticker="NQ", contract_size=20, dte_max=None)
    assert np.isfinite(a.total_gex)
    assert a.total_gex != 0.0
