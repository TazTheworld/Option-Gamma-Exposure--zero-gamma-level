"""Parsers de sources : CME (fichier), Databento (frames) et flux CBOE.

Tous les jeux d'essai sont construits depuis des paramètres connus, ce qui permet
de vérifier qu'on retrouve la vérité terrain plutôt que de figer une sortie.
Aucun test ne touche au réseau.
"""

import numpy as np
import pandas as pd
import pytest

import cboe_data
import cme_data
import databento_data as dbd
import flow_tracker as ft

F_VRAI, VOL_VRAI = 1.06500, 0.0700
EXPIRY, QUOTE = "2026-09-04", "2026-08-04"
T = (pd.Timestamp(EXPIRY) + pd.Timedelta(hours=16) - pd.Timestamp(QUOTE)).total_seconds() / (365.25 * 24 * 3600)
STRIKES = np.round(np.arange(1.00, 1.135, 0.005), 3)


def _prix(k, cp):
    return round(cme_data.black76_price(F_VRAI, k, VOL_VRAI, T, 0.0, cp), 5)


# ---=== cboe_data ===---

@pytest.mark.parametrize("entree,attendu", [
    ("SPCX", "SPCX"), ("spcx", "SPCX"), ("SPX", "_SPX"),
    ("_SPX", "_SPX"), ("^SPX", "_SPX"), ("VIX", "_VIX"),
])
def test_symbole_cboe_prefixe_les_indices(entree, attendu):
    assert cboe_data.cboe_symbol(entree) == attendu


def test_regex_symbole_occ_decoupe_strike_et_echeance():
    m = cboe_data._OPTION_RE.match("SPCX260807C00005000")
    assert m["exp"] == "260807" and m["cp"] == "C"
    assert int(m["strike"]) / 1000 == 5.0
    # un root contenant des chiffres ne doit pas décaler le découpage
    m2 = cboe_data._OPTION_RE.match("1SPCX260807P00330000")
    assert m2["cp"] == "P" and int(m2["strike"]) / 1000 == 330.0


# ---=== cme_data ===---

def _ecrire_cme(tmp_path, large, avec_iv):
    """Fabrique un export CME synthétique, au format large ou long."""
    if large:
        chemin = tmp_path / "cme_large.csv"
        pd.DataFrame({
            "Call Settlement Price": [_prix(k, "C") for k in STRIKES],
            "Call Prior Day OI": np.arange(100, 100 + len(STRIKES)),
            "Strike Price": STRIKES,
            "Put Settlement Price": [_prix(k, "P") for k in STRIKES],
            "Put Prior Day OI": np.arange(200, 200 + len(STRIKES)),
        }).to_csv(chemin, index=False)
    else:
        chemin = tmp_path / "cme_long.csv"
        lignes = []
        for k in STRIKES:
            for typ, cp in (("Call", "C"), ("Put", "P")):
                ligne = {"Strike": k, "Type": typ, "Settle": _prix(k, cp),
                         "Open Interest": 500, "Expiration": EXPIRY}
                if avec_iv:
                    ligne["Implied Volatility"] = VOL_VRAI * 100   # le CME publie en %
                lignes.append(ligne)
        pd.DataFrame(lignes).to_csv(chemin, index=False)
    return chemin


@pytest.mark.parametrize("large,avec_iv", [(True, False), (False, True), (False, False)])
def test_cme_retrouve_le_prix_du_future_par_parite(tmp_path, large, avec_iv):
    """F = K + (C - P) au strike le plus ATM, à r = 0."""
    kw = {"expiry": EXPIRY} if large else {}
    _, prix, _ = cme_data.load_settlement(_ecrire_cme(tmp_path, large, avec_iv),
                                          quote_date=QUOTE, **kw)
    assert prix == pytest.approx(F_VRAI, abs=1e-4)


@pytest.mark.parametrize("large,avec_iv", [(True, False), (False, True), (False, False)])
def test_cme_reconstitue_iv_et_gamma(tmp_path, large, avec_iv):
    """IV lue du fichier si présente, sinon inversée depuis le prix de règlement."""
    kw = {"expiry": EXPIRY} if large else {}
    chaine, prix, _ = cme_data.load_settlement(_ecrire_cme(tmp_path, large, avec_iv),
                                               quote_date=QUOTE, **kw)
    assert len(chaine) == len(STRIKES)
    # tolérance large : les prix du fichier sont arrondis à 5 décimales
    assert chaine.CallIV.mean() == pytest.approx(VOL_VRAI, abs=1e-3)
    assert (chaine.CallGamma > 0).all()
    assert chaine.CallGamma.max() == pytest.approx(chaine.PutGamma.max(), rel=1e-6)


def test_cme_prix_du_future_explicite_prime_sur_la_parite(tmp_path):
    _, prix, _ = cme_data.load_settlement(_ecrire_cme(tmp_path, False, True),
                                          futures_price=1.2345, quote_date=QUOTE)
    assert prix == 1.2345


def test_cme_colonne_strike_absente_leve_une_erreur_lisible(tmp_path):
    chemin = tmp_path / "sans_strike.csv"
    pd.DataFrame({"Machin": [1], "Truc": [2]}).to_csv(chemin, index=False)
    with pytest.raises(ValueError, match="strike"):
        cme_data.load_settlement(chemin, quote_date=QUOTE)


def test_cme_alias_latest_de_barchart_reconnu(tmp_path):
    """Barchart nomme sa colonne de prix 'Latest' et non 'Settle'."""
    chemin = tmp_path / "barchart.csv"
    pd.DataFrame([
        {"Strike": k, "Type": typ, "Latest": _prix(k, typ[0]),
         "Open Interest": 500, "IV": VOL_VRAI * 100, "Expiration": EXPIRY}
        for k in STRIKES for typ in ("Call", "Put")
    ]).to_csv(chemin, index=False)
    _, prix, _ = cme_data.load_settlement(chemin, quote_date=QUOTE)
    assert prix == pytest.approx(F_VRAI, abs=1e-4)


# ---=== databento_data ===---

def _frames_databento(strikes_en_entier_fixe):
    """Imite la sortie de .to_df() : definitions + statistics."""
    from databento_dbn import StatType
    exp = pd.Timestamp("2026-09-04 14:00")
    defs, stats, iid = [], [], 1000
    for k in STRIKES:
        for cp in ("C", "P"):
            defs.append({"instrument_id": iid, "raw_symbol": f"6E{cp}{k}",
                         "instrument_class": cp, "expiration": exp,
                         "strike_price": int(k * 1e9) if strikes_en_entier_fixe else k})
            stats += [
                {"instrument_id": iid, "stat_type": int(StatType.SETTLEMENT_PRICE),
                 "price": _prix(k, cp), "quantity": np.nan},
                {"instrument_id": iid, "stat_type": int(StatType.OPEN_INTEREST),
                 "price": np.nan, "quantity": 500},
            ]
            iid += 1
    # bruit : un future et un spread, qui ne sont pas des options
    defs += [{"instrument_id": 9999, "raw_symbol": "6EU6", "instrument_class": "F",
              "strike_price": 0, "expiration": exp},
             {"instrument_id": 9998, "raw_symbol": "6E-SPD", "instrument_class": "T",
              "strike_price": 0, "expiration": exp}]
    return pd.DataFrame(defs), pd.DataFrame(stats)


@pytest.mark.parametrize("entier_fixe", [True, False])
def test_databento_assemble_la_chaine(entier_fixe):
    chaine, prix, _ = dbd.build_chain(*_frames_databento(entier_fixe), quote_date=QUOTE)
    assert len(chaine) == len(STRIKES)          # futures et spreads écartés
    assert prix == pytest.approx(F_VRAI, abs=1e-4)
    assert chaine.CallIV.mean() == pytest.approx(VOL_VRAI, abs=1e-3)
    assert chaine.CallOpenInt.sum() == 500 * len(STRIKES)


def test_databento_codes_stat_lus_depuis_l_enumeration():
    """La doc publique donnait des valeurs fausses : on lit le paquet, pas la doc."""
    from databento_dbn import StatType
    codes = dbd._stat_types()
    assert codes["oi"] == int(StatType.OPEN_INTEREST)
    assert codes["settle"] == int(StatType.SETTLEMENT_PRICE)


def test_databento_definitions_vides_leve_une_erreur():
    with pytest.raises(ValueError):
        dbd.build_chain(pd.DataFrame({"instrument_class": []}), pd.DataFrame({"stat_type": []}))


# ---=== flow_tracker ===---

T0, T1 = pd.Timestamp("2026-08-07 14:00"), pd.Timestamp("2026-08-07 14:05")


def _releve(volumes, derniers, horodatages):
    return pd.DataFrame({
        "option": [f"O{i}" for i in range(len(volumes))],
        "StrikePrice": [150.0, 150.0, 140.0, 160.0, 145.0],
        "cp": ["C", "P", "C", "C", "P"], "expiry": ["260814"] * 5,
        "volume": volumes, "bid": [1.00] * 5, "ask": [2.00] * 5,
        "last": derniers, "last_time": pd.to_datetime(horodatages)})


def test_flux_classe_selon_la_position_dans_la_fourchette():
    avant = _releve([100, 50, 200, 10, 0], [1.5] * 5, [T0] * 5)
    apres = _releve([500, 150, 500, 10, 50],
                    [1.95, 1.05, 1.50, 1.5, 1.95],
                    [T1, T1, T1, T0, T1])
    flux = ft.classify(avant, apres)
    cotes = dict(zip(flux.option, flux.cote.astype(str)))
    assert cotes == {"O0": "achat", "O1": "vente", "O2": "milieu", "O4": "achat"}
    assert "O3" not in cotes                      # volume inchangé
    assert flux.loc[flux.option == "O0", "prime"].iloc[0] == pytest.approx(400 * 1.95 * 100)


def test_flux_ignore_les_trades_dont_l_horodatage_n_a_pas_avance():
    """Sans avancée de last_time, la fourchette n'est pas contemporaine du trade."""
    avant = _releve([100] * 5, [1.5] * 5, [T1] * 5)
    apres = _releve([500] * 5, [1.95] * 5, [T1] * 5)   # volume monte, horodatage figé
    assert ft.classify(avant, apres).empty


def test_flux_etat_marche_convertit_utc_vers_new_york():
    """L'horodatage du payload CBOE est en UTC, celui des trades en heure de NY."""
    ouvert, et = ft.etat_marche("2026-08-07 13:20:00")   # 09:20 ET, avant l'ouverture
    assert not ouvert and et.hour == 9 and et.minute == 20
    ouvert, et = ft.etat_marche("2026-08-07 17:00:00")   # 13:00 ET, en séance
    assert ouvert
    assert not ft.etat_marche("2026-08-08 17:00:00")[0]  # samedi


@pytest.mark.parametrize("texte,secondes", [("6h", 21600), ("90m", 5400), ("300", 300), ("1.5h", 5400)])
def test_parsing_des_durees(texte, secondes):
    assert ft._duree(texte) == secondes
