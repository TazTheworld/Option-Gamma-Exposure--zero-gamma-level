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


def _ecrire(fichier, n=40, ratio_vol=3.0, graine=7, dte_max=30, source="iv",
            depart=100.0, debut="2026-06-01"):
    """Historique synthétique : régimes alternés, amplitude ratio_vol fois plus
    grande en gamma négatif."""
    rng = np.random.default_rng(graine)
    spot = depart
    for i in range(n):
        gex = -50e6 if i % 2 == 0 else +50e6
        vol = 0.01 * (ratio_vol if gex < 0 else 1.0)
        history.record(fichier, timestamp=pd.Timestamp(debut) + pd.Timedelta(days=i),
                       ticker="TEST", dte_max=dte_max, source_gamma=source,
                       spot=spot, total_gex=gex,
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


# ---=== Périmètres et séances ===---

def test_un_historique_ecrit_par_une_version_anterieure_est_migre(fichier, tmp_path):
    """Ajouter une colonne ne doit pas décaler silencieusement les lignes existantes."""
    anciennes = [c for c in history.COLONNES if c != "source_gamma"]
    pd.DataFrame([{c: 1 for c in anciennes}]).to_csv(fichier, index=False)

    history.record(fichier, ticker="AAA", dte_max=30, source_gamma="iv", spot=123.0)
    df = pd.read_csv(fichier)
    assert list(df.columns) == history.COLONNES
    assert len(df) == 2
    assert df.spot.iloc[0] == 1 and df.spot.iloc[1] == 123.0   # rien n'a glissé
    assert pd.isna(df.source_gamma.iloc[0])


def test_dedoublonner_ne_garde_qu_une_seance(fichier):
    """Trois exécutions le même jour, c'est une observation, pas trois."""
    for heure, spot in (("09:45", 100.0), ("12:00", 100.5), ("16:00", 101.0)):
        history.record(fichier, timestamp=f"2026-06-01 {heure}", ticker="AAA",
                       dte_max=30, source_gamma="iv", spot=spot, total_gex=1e6)
    seances = validate.dedoublonner(history.load(fichier))
    assert len(seances) == 1
    assert seances.spot.iloc[0] == 101.0        # la dernière du jour


def test_prepare_rejette_les_intervalles_intraday(fichier):
    """20 minutes d'écart annonçaient 1,7 % / jour pour un mouvement réel de 0,2 %."""
    for horodatage, spot in (("2026-06-01 14:00", 100.0), ("2026-06-01 14:20", 100.2),
                             ("2026-06-02 16:00", 101.0)):
        history.record(fichier, timestamp=horodatage, ticker="AAA", dte_max=30,
                       source_gamma="iv", spot=spot, total_gex=-1e6, zero_gamma=99.0)
    df = validate.prepare(history.load(fichier))
    assert len(df) == 1
    assert df.jours.iloc[0] >= validate.INTERVALLE_MINIMAL
    assert df.mouvement_par_jour.iloc[0] < 0.02      # plus de 1,7 % fantôme


def test_validate_separe_les_perimetres(fichier, capsys):
    """Deux horizons dans le même fichier = deux mesures, pas une série mélangée.

    Le GEX peut changer de signe rien qu'en changeant de dte_max : les enchaîner
    classait le régime d'après une grandeur qui n'était pas la même d'une ligne
    à l'autre.
    """
    _ecrire(fichier, n=10, dte_max=30, source="iv")
    _ecrire(fichier, n=10, dte_max=7, source="iv", graine=11, depart=200.0)
    validate.valider(fichier)
    sortie = capsys.readouterr().out
    assert "dte_max=30" in sortie and "dte_max=7" in sortie
    assert sortie.count("AMPLITUDE SELON LE RÉGIME") == 2


def test_validate_separe_les_sources_de_gamma(fichier, capsys):
    """13 % d'écart entre les deux sources sur un indice : ce ne sont pas les mêmes séries."""
    _ecrire(fichier, n=8, source="iv")
    _ecrire(fichier, n=8, source="published", graine=3, depart=150.0)
    validate.valider(fichier)
    sortie = capsys.readouterr().out
    assert "gamma=iv" in sortie and "gamma=published" in sortie


def test_show_separe_les_perimetres(fichier, capsys):
    _ecrire(fichier, n=4, dte_max=30)
    _ecrire(fichier, n=4, dte_max=7, graine=2)
    history.show(fichier, "TEST")
    sortie = capsys.readouterr().out
    assert "périmètre dte_max=30" in sortie and "périmètre dte_max=7" in sortie


def test_show_vue_d_ensemble_une_ligne_par_perimetre(fichier, capsys):
    _ecrire(fichier, n=3, dte_max=30)
    _ecrire(fichier, n=3, dte_max=7, graine=2)
    vue = history.show(fichier)
    assert len(vue) == 2
    assert set(vue.dte_max) == {30, 7}


# ---=== Contexte de séance : high/low et volatilité implicite ===---

def test_vol_parkinson_retrouve_une_amplitude_connue():
    """sigma = ln(H/L) / (2 racine(ln 2)), annualisé sur 252 séances."""
    attendu = np.log(110 / 100) / (2 * np.sqrt(np.log(2))) * np.sqrt(252)
    obtenu = validate.vol_parkinson(pd.Series([110.0]), pd.Series([100.0])).iloc[0]
    assert obtenu == pytest.approx(attendu)
    # Une séance sans amplitude n'a pas de volatilité, pas une volatilité nulle bruitée
    assert validate.vol_parkinson(pd.Series([100.0]), pd.Series([100.0])).iloc[0] == 0.0
    assert np.isnan(validate.vol_parkinson(pd.Series([90.0]), pd.Series([100.0])).iloc[0])


def _ecrire_seances(fichier, seances):
    """seances : liste de (spot, high, low, close, gex, iv30, call_wall)."""
    for i, (spot, haut, bas, cloture, gex, iv30, mur) in enumerate(seances):
        history.record(fichier, timestamp=pd.Timestamp("2026-06-01") + pd.Timedelta(days=i),
                       ticker="TEST", dte_max=30, source_gamma="iv",
                       time_convention="heures", spot=spot, total_gex=gex,
                       zero_gamma=spot, call_wall=mur, put_wall=spot * 0.9,
                       high=haut, low=bas, close=cloture, iv30=iv30)
    return fichier


def test_le_mur_touche_en_seance_ne_compte_pas_comme_tenu(fichier, capsys):
    """Un mur percé puis rejeté ressortait comme respecté : la clôture ne le voyait pas.

    Ici le prix va chercher le mur à 104 chaque séance (high 105) mais clôture
    toujours en dessous — c'est exactement le comportement que le modèle prédit,
    et la mesure doit savoir le distinguer d'un mur jamais approché.
    """
    _ecrire_seances(fichier, [(100.0, 105.0, 99.0, 101.0, -1e6, 30.0, 104.0)] * 6)
    validate.valider(fichier)
    sortie = capsys.readouterr().out
    assert "touché en séance 100% du temps" in sortie
    assert "tenu à la clôture 0%" in sortie
    assert "rejeté après avoir été touché : 100%" in sortie


def test_realise_contre_implicite_detecte_le_signal_injecte(fichier, capsys):
    """Le relevé décrit un régime, la volatilité mesurée est celle de la séance SUIVANTE.

    Le signal doit donc être injecté décalé d'un cran : un relevé en gamma négatif
    est suivi d'une séance à large amplitude. Injecté sur la même ligne, il mesure
    exactement l'inverse — et le test passerait sur une implémentation fausse.
    """
    seances = []
    for i in range(26):                           # au-delà des 20 intervalles exigés
        # Le gamma est négatif un relevé sur deux ; l'amplitude large arrive au
        # relevé d'après, soit sur les indices impairs.
        gex = -1e6 if i % 2 == 0 else 1e6
        amplitude = 0.12 if i % 2 else 0.01       # 12 % de range contre 1 %
        spot = 100.0 + i * 0.1                    # un spot figé rend le test d'amplitude indéfini
        seances.append((spot, spot * (1 + amplitude), spot, spot, gex, 30.0, 130.0))
    _ecrire_seances(fichier, seances)
    validate.valider(fichier)
    sortie = capsys.readouterr().out
    assert "RÉALISÉ CONTRE IMPLICITE" in sortie
    assert "plus souvent en gamma négatif" in sortie
    assert "conforme au modèle" in sortie


def test_realise_contre_implicite_ne_confirme_pas_un_signal_inverse(fichier, capsys):
    """Signal retourné : le test doit dire CONTRAIRE, pas confirmer par défaut."""
    seances = []
    for i in range(26):
        gex = -1e6 if i % 2 == 0 else 1e6
        amplitude = 0.01 if i % 2 else 0.12       # l'amplitude suit le gamma POSITIF
        spot = 100.0 + i * 0.1
        seances.append((spot, spot * (1 + amplitude), spot, spot, gex, 30.0, 130.0))
    _ecrire_seances(fichier, seances)
    validate.valider(fichier)
    assert "CONTRAIRE au modèle" in capsys.readouterr().out


def test_sans_contexte_de_seance_le_test_le_dit_au_lieu_d_inventer(fichier, capsys):
    """Les relevés d'avant l'archivage du contexte n'ont ni high/low ni iv30."""
    _ecrire(fichier, n=10)
    validate.valider(fichier)
    sortie = capsys.readouterr().out
    assert "il en faut plus" in sortie or "Ces colonnes n'existent" in sortie
    assert "mesure sur la clôture seule" in sortie
