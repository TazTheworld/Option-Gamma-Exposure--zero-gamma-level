"""Historique et validation.

Le validateur est lui-même testé sur un signal injecté : on fabrique un historique
où le gamma négatif produit vraiment plus d'amplitude, et on vérifie qu'il le
détecte. Un validateur qui ne sait rien trouver ne prouverait rien.
"""

import numpy as np
import pandas as pd
import pytest

import history
import validate


@pytest.fixture
def fichier(tmp_path):
    return str(tmp_path / "h.csv")


def _ecrire(fichier, n=40, ratio_vol=3.0, graine=7):
    """Historique synthétique : régimes alternés, amplitude ratio_vol fois plus
    grande en gamma négatif."""
    rng = np.random.default_rng(graine)
    spot = 100.0
    for i in range(n):
        gex = -50e6 if i % 2 == 0 else +50e6
        vol = 0.01 * (ratio_vol if gex < 0 else 1.0)
        history.record(fichier, timestamp=pd.Timestamp("2026-06-01") + pd.Timedelta(days=i),
                       ticker="TEST", dte_max=30, spot=spot, total_gex=gex,
                       zero_gamma=spot * (1.05 if gex < 0 else 0.95),
                       call_wall=spot * 1.04, put_wall=spot * 0.96,
                       call_wall_oi=spot * 1.1, put_wall_oi=spot * 0.9,
                       charm=-1e6, vanna=1e6, strikes=50, expiries=4)
        spot *= 1 + rng.normal(0, vol)
    return fichier


def test_record_cree_puis_complete_le_fichier(fichier):
    history.record(fichier, ticker="AAA", spot=1.0)
    history.record(fichier, ticker="BBB", spot=2.0)
    df = pd.read_csv(fichier)
    assert len(df) == 2
    assert list(df.columns) == history.COLONNES     # en-tête écrit une seule fois
    assert set(df.ticker) == {"AAA", "BBB"}


def test_record_horodate_automatiquement(fichier):
    history.record(fichier, ticker="AAA", spot=1.0)
    assert pd.read_csv(fichier).timestamp.notna().all()


def test_load_filtre_par_ticker_sans_tenir_compte_de_la_casse(fichier):
    history.record(fichier, ticker="SPCX", spot=1.0)
    history.record(fichier, ticker="ORCL", spot=2.0)
    assert len(history.load(fichier, "spcx")) == 1
    assert len(history.load(fichier, "_SPCX")) == 1      # underscore d'indice toléré
    assert len(history.load(fichier)) == 2


def test_load_sans_fichier_donne_une_erreur_actionnable(tmp_path):
    with pytest.raises(FileNotFoundError, match="main.py"):
        history.load(str(tmp_path / "absent.csv"))


def test_compact_abrege_les_grands_nombres():
    assert history._compact(58_107_413) == "+58.1M"
    assert history._compact(-49_253_080_615) == "-49.25Md"
    assert history._compact(np.nan) == "-"


def test_show_ne_plante_pas_sur_un_seul_releve(fichier, capsys):
    history.record(fichier, ticker="AAA", spot=1.0, total_gex=1e6)
    history.show(fichier, "AAA")
    assert "AAA" in capsys.readouterr().out


def test_validate_detecte_le_signal_injecte(fichier, capsys):
    """Amplitude 3x plus grande en gamma négatif : le script doit la voir."""
    validate.valider(_ecrire(fichier, ratio_vol=3.0))
    sortie = capsys.readouterr().out
    assert "plus amples en gamma négatif" in sortie
    assert "conforme au modèle" in sortie


def test_validate_signale_un_resultat_contraire(fichier, capsys):
    """Signal inversé : le validateur ne doit pas confirmer le modèle par défaut."""
    validate.valider(_ecrire(fichier, ratio_vol=0.33))
    sortie = capsys.readouterr().out
    assert "plus faibles en gamma négatif" in sortie
    assert "CONTRAIRE au modèle" in sortie


def test_validate_avertit_sur_echantillon_court(fichier, capsys):
    validate.valider(_ecrire(fichier, n=8))
    sortie = capsys.readouterr().out
    assert "ATTENTION" in sortie and "indicatifs" in sortie


def test_validate_gere_un_historique_trop_court(fichier, capsys):
    history.record(fichier, ticker="AAA", spot=1.0, total_gex=1e6)
    validate.valider(fichier)
    assert "pas assez de relevés" in capsys.readouterr().out


def test_prepare_ignore_les_intervalles_nuls(fichier):
    """Deux relevés au même horodatage ne forment pas un intervalle."""
    for _ in range(2):
        history.record(fichier, timestamp="2026-06-01 16:00", ticker="AAA",
                       spot=100.0, total_gex=1e6)
    assert validate.prepare(history.load(fichier)).empty
