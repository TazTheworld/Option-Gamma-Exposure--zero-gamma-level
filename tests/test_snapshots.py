"""Archivage des chaînes brutes : ce que le collecteur écrit, ce qui se relit.

Ces tests vivaient dans `tests/test_analysis.py`, qui couvrait à la fois le moteur
de calcul et l'archivage. Le moteur est passé en Rust ; l'archivage reste en
Python tant que le collecteur y est. Les séparer évite que la suppression de l'un
emporte la couverture de l'autre.

Le format archivé est le joint entre les deux mondes : `ib_collector.py` écrit,
`gex-store` lit. Un changement de colonne ici casserait le moteur Rust sans
qu'aucun test Rust ne le voie, puisqu'il travaille sur une fixture figée.
"""

import shutil

import numpy as np
import pandas as pd
import pytest

import chain
import snapshots
from chain import COLUMNS

SPOT = 100.0
QUOTE = pd.Timestamp("2026-08-12 16:00")


def chaine(jours=(30,), strikes=None):
    """Chaîne synthétique au format COLUMNS.

    L'open interest est volontairement ASYMÉTRIQUE entre calls et puts : à OI
    égal les deux jambes se compensent exactement, et un aller-retour d'archive
    passerait sur du vide sans rien prouver.
    """
    strikes = np.arange(90.0, 111.0, 5.0) if strikes is None else np.asarray(strikes, float)
    lignes = []
    for j in jours:
        # 16h00 le jour de l'échéance, pas 16h après le relevé : c'est l'heure
        # de règlement, et c'est elle qui doit survivre à l'aller-retour.
        echeance = QUOTE.normalize() + pd.Timedelta(days=int(j)) + pd.Timedelta(hours=16)
        for k in strikes:
            lignes.append({
                "ExpirationDate": echeance, "StrikePrice": float(k),
                "CallIV": 0.20, "PutIV": 0.22,
                "CallGamma": 0.03, "PutGamma": 0.03,
                "CallOpenInt": 300.0, "PutOpenInt": 900.0,
                "CallDelta": 0.5, "PutDelta": -0.5,
            })
    df = pd.DataFrame(lignes)
    for colonne in COLUMNS + chain.COLONNES_GRECS:
        if colonne not in df.columns:
            df[colonne] = 0.0
    return df[COLUMNS + chain.COLONNES_GRECS]


def test_snapshot_aller_retour(tmp_path):
    """Une séance archivée doit se relire à l'identique — c'est la raison d'être
    de l'archive, et le contrat que le moteur Rust suppose en ouvrant le fichier."""
    df = chaine(jours=(1, 8, 25))
    chemin = snapshots.sauver(df, "TEST", SPOT, QUOTE, str(tmp_path))
    relu, spot, quote_date, marche = snapshots.charger(chemin)

    assert spot == SPOT and pd.Timestamp(quote_date) == QUOTE
    assert marche == {}                     # rien n'a été archivé, rien n'est inventé
    assert len(relu) == len(df)
    for colonne in ("StrikePrice", "CallIV", "PutIV", "CallOpenInt", "PutOpenInt"):
        assert relu[colonne].tolist() == pytest.approx(df[colonne].tolist())
    assert relu.ExpirationDate.tolist() == df.ExpirationDate.tolist()


def test_l_heure_de_reglement_survit_a_l_archivage(tmp_path):
    """Le défaut qui a coûté 19 % du GEX : une échéance sans heure fait passer le
    décalage de fuseau pour du temps restant. L'archive doit la transporter."""
    df = chaine(jours=(1,))
    chemin = snapshots.sauver(df, "TEST", SPOT, QUOTE, str(tmp_path))
    relu, _, _, _ = snapshots.charger(chemin)
    assert relu.ExpirationDate.dt.hour.unique().tolist() == [16]


def test_snapshot_transporte_le_contexte_de_marche(tmp_path):
    """Sans le contexte archivé, une séance rejouée perd le ratio au volume."""
    marche = {"open": 135.0, "high": 146.1, "low": 134.0, "close": 144.9,
              "volume": 95_310_234.0, "dollar_volume": 1.3808e10, "iv30": 70.2}
    chemin = snapshots.sauver(chaine(), "TEST", SPOT, QUOTE, str(tmp_path), marche)
    _, _, _, relu = snapshots.charger(chemin)
    assert relu == pytest.approx(marche)


def test_snapshot_lister_et_dernier(tmp_path):
    df = chaine()
    snapshots.sauver(df, "AAA", SPOT, "2026-08-10 16:00", str(tmp_path))
    dernier = snapshots.sauver(df, "AAA", SPOT, "2026-08-11 16:00", str(tmp_path))
    snapshots.sauver(df, "BBB", SPOT, "2026-08-11 16:00", str(tmp_path))
    assert len(snapshots.lister("AAA", str(tmp_path))) == 2
    assert len(snapshots.lister(dossier=str(tmp_path))) == 3
    assert snapshots.dernier("AAA", str(tmp_path)) == dernier
    assert snapshots.dernier("ZZZ", str(tmp_path)) is None


def test_snapshot_courant_a_un_chemin_fixe(tmp_path):
    """Le relevé vivant est réécrit en place : sinon 1 500 fichiers par jour."""
    un = snapshots.courant("NQ", str(tmp_path))
    deux = snapshots.courant("nq", str(tmp_path))
    assert un == deux                       # insensible à la casse, comme chemin()
    assert un.endswith(("courant.parquet", "courant.csv.gz"))
    assert "NQ" in un


def test_snapshot_lister_ignore_le_courant(tmp_path):
    """Le courant n'est pas une archive : il ne doit pas remonter dans un rejeu."""
    archive = snapshots.sauver(chaine(), "NQ", SPOT, QUOTE, str(tmp_path))
    vivant = snapshots.courant("NQ", str(tmp_path))
    shutil.copy(archive, vivant)            # le courant existe sur disque

    trouves = snapshots.lister("NQ", str(tmp_path))
    assert vivant not in trouves
    assert archive in trouves
    assert snapshots.dernier("NQ", str(tmp_path)) != vivant


def test_snapshot_courant_relu_comme_une_archive(tmp_path):
    """Même format que les archives : le lecteur doit pouvoir ouvrir l'un ou
    l'autre sans savoir lequel il tient."""
    df = chaine(jours=(1, 8))
    archive = snapshots.sauver(df, "NQ", SPOT, QUOTE, str(tmp_path))
    cible = snapshots.courant("NQ", str(tmp_path))
    shutil.copy(archive, cible)

    relu, spot, quote_date, marche = snapshots.charger(cible)
    assert spot == SPOT and pd.Timestamp(quote_date) == QUOTE
    assert len(relu) == len(df)


def test_charger_un_fichier_qui_n_est_pas_un_releve(tmp_path):
    """Échouer bruyamment vaut mieux que rendre du vide : une chaîne vide donnerait
    un GEX de zéro, qui est un chiffre et non une erreur."""
    faux = tmp_path / "faux.csv.gz"
    pd.DataFrame({"a": [1]}).to_csv(faux, index=False, compression="gzip")
    with pytest.raises(ValueError, match="relevé archivé"):
        snapshots.charger(str(faux))


def test_charger_un_fichier_absent(tmp_path):
    with pytest.raises(FileNotFoundError):
        snapshots.charger(str(tmp_path / "absent.parquet"))
