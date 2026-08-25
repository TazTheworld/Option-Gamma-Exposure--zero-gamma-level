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
