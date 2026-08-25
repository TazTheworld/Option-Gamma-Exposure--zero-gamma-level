"""Les fonctions pures du collecteur Interactive Brokers.

Les jeux d'essai sont fabriqués depuis des paramètres
connus : on vérifie qu'on retrouve la vérité terrain, pas qu'une sortie est figée.

Aucun test ne touche au réseau, et rien ici n'importe ib_async — la dépendance de
courtier est un extra, le cœur du projet tient à cinq paquets.
"""

import numpy as np
import pandas as pd
import pytest

import ib_data
from chain import COLUMNS

QUOTE = pd.Timestamp("2026-08-25")
PRIX = 25_000.0
STRIKES = np.arange(20_000.0, 30_025.0, 25.0)
ECHEANCES = ["20260826", "20260828", "20260904", "20260918", "20261120"]


# ---=== echeances_utiles ===---

def test_echeances_utiles_coupe_l_horizon():
    """dte_max=30 écarte le 20 novembre, dte_min=2 écarte le lendemain."""
    gardees = ib_data.echeances_utiles(ECHEANCES, QUOTE, dte_max=30, dte_min=2)
    assert [d.strftime("%Y%m%d") for d in gardees] == ["20260828", "20260904", "20260918"]


def test_echeances_utiles_dte_max_none_garde_tout():
    """Le 'all' de main.py arrive ici en None : aucune échéance ne doit tomber."""
    assert len(ib_data.echeances_utiles(ECHEANCES, QUOTE, dte_max=None)) == len(ECHEANCES)


def test_echeances_utiles_dedoublonne_et_trie():
    """C'est le nombre d'appels a reqContractDetails : un doublon est une requête
    payée pour rien, et l'ordre doit être reproductible."""
    gardees = ib_data.echeances_utiles(["20260904", "20260904", "20260828"], QUOTE)
    assert [d.strftime("%Y%m%d") for d in gardees] == ["20260828", "20260904"]


def test_echeances_utiles_vide_sans_erreur():
    """Un horizon qui ne contient rien n'est pas une panne."""
    assert ib_data.echeances_utiles(ECHEANCES, QUOTE, dte_max=0) == []


# ---=== perimetre ===---

def _contrats_cotes(strikes=None, echeance="2026-09-04"):
    """Ce que reqContractDetails rend : les contrats REELLEMENT cotes, avec conId."""
    strikes = np.arange(24_375.0, 25_650.0, 25.0) if strikes is None else strikes
    lignes = []
    for i, k in enumerate(strikes):
        for j, r in enumerate(("C", "P")):
            lignes.append({"conId": 200_000 + i * 2 + j, "StrikePrice": float(k),
                           "ExpirationDate": pd.Timestamp(echeance), "right": r})
    return pd.DataFrame(lignes)


def test_perimetre_coupe_les_strikes_hors_plage():
    """±2 % autour de 25 000, c'est 24 500 à 25 500 et rien d'autre."""
    p = ib_data.perimetre(_contrats_cotes(), PRIX, plage=0.02)
    assert p.StrikePrice.min() == pytest.approx(24_500.0)
    assert p.StrikePrice.max() == pytest.approx(25_500.0)


def test_perimetre_garde_le_strike_exactement_a_la_borne():
    """25 000 x 1,025 vaut 25 624,999999999996 en flottant : sans marge, le strike
    25 625 pourtant demandé tombe — et de façon variable selon le prix."""
    p = ib_data.perimetre(_contrats_cotes(), PRIX, plage=0.025)
    assert 25_625.0 in set(p.StrikePrice)
    assert 24_375.0 in set(p.StrikePrice)


def test_perimetre_conserve_les_conid():
    """C'est la seule chose que la couche réseau ne peut pas reconstruire."""
    p = ib_data.perimetre(_contrats_cotes(), PRIX, plage=0.01)
    assert "conId" in p.columns
    assert p.conId.is_unique


def test_perimetre_ne_fabrique_aucun_contrat():
    """Le fond de la correction : on filtre ce qui est coté, on n'invente pas de
    couple. reqSecDefOptParams rend l'union des strikes et des échéances, pas les
    contrats existants — leur produit cartésien est un majorant, pas une chaîne."""
    cotes = _contrats_cotes(strikes=[24_900.0, 25_000.0, 25_100.0])
    p = ib_data.perimetre(cotes, PRIX, plage=0.2)
    assert len(p) == len(cotes)              # rien de plus que ce qui est coté
    assert set(p.conId) <= set(cotes.conId)


def test_perimetre_garde_les_deux_cotes():
    """Le GEX a besoin des deux jambes."""
    p = ib_data.perimetre(_contrats_cotes(strikes=[25_000.0]), PRIX, plage=0.01)
    assert sorted(p.right) == ["C", "P"]


def test_perimetre_vide_rend_une_trame_typee_pas_une_erreur():
    """Une échéance dont aucun strike n'est dans la plage n'est pas une panne."""
    p = ib_data.perimetre(_contrats_cotes(), prix=1.0, plage=0.02)
    assert p.empty
    assert "StrikePrice" in p.columns and "conId" in p.columns


def test_perimetre_trie_pour_que_les_lots_soient_reproductibles():
    """L'ordre est celui dans lequel les lots partiront : deux appels identiques
    doivent rendre la même liste, sinon le recyclage re-souscrit pour rien."""
    cotes = _contrats_cotes()
    un = ib_data.perimetre(cotes, PRIX, plage=0.01)
    deux = ib_data.perimetre(cotes.sample(frac=1, random_state=0), PRIX, plage=0.01)
    assert un.equals(deux)
    assert un.StrikePrice.is_monotonic_increasing


# ---=== build_chain ===---

VOL_VRAI = 0.18
EXP = pd.Timestamp("2026-09-04 14:00")
T_VRAI = (EXP - QUOTE).total_seconds() / (365.25 * 24 * 3600)


def _trames_ib(avec_gamma=True, iv_en_pourcent=False, oi=500.0):
    """Imite ce que la couche réseau produira : définitions + valeurs par conId."""
    from black76 import black76_gamma
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
    from black76 import black76_gamma
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
    from black76 import black76_price
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


# ---=== selection_vif ===---

def test_selection_vif_respecte_le_budget_de_lignes():
    """Cent lignes chez IB, quatre-vingt-dix pour le vif : saturer le quota fait
    échouer les souscriptions suivantes en silence."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert len(ib_data.selection_vif(chaine, budget_lignes=90)) <= 90
    assert len(ib_data.selection_vif(chaine, budget_lignes=10)) == 10


def test_selection_vif_prend_les_contrats_qui_portent_le_gamma():
    """Le socle vient de mesurer où le gamma est : un critère géométrique
    gaspillerait des lignes sur des strikes sans open interest."""
    defs, ticks = _trames_ib(oi=1.0)
    # un seul strike porte tout l'open interest, et il est loin de la monnaie
    loin = defs[(defs.StrikePrice == 24_600.0) & (defs.right == "C")].conId.iloc[0]
    ticks = ticks.copy()
    ticks.loc[ticks.conId == loin, "OpenInt"] = 1_000_000.0
    chaine, _, _ = ib_data.build_chain(defs, ticks, futures_price=PRIX,
                                       quote_date=QUOTE)

    choisis = ib_data.selection_vif(chaine, budget_lignes=3)
    premier = choisis.iloc[0]
    assert premier.StrikePrice == pytest.approx(24_600.0)
    assert premier.right == "C"


def test_selection_vif_ecarte_les_contrats_sans_poids():
    """Un strike sans open interest ne mérite pas une ligne."""
    defs, ticks = _trames_ib(oi=0.0)
    chaine, _, _ = ib_data.build_chain(defs, ticks, futures_price=PRIX,
                                       quote_date=QUOTE)
    assert ib_data.selection_vif(chaine).empty


def test_selection_vif_rend_la_meme_forme_que_perimetre():
    """La couche réseau ne doit connaître qu'une seule forme de contrat."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert list(ib_data.selection_vif(chaine).columns) == ib_data.CONTRAT


def test_selection_vif_est_deterministe_a_egalite():
    """À poids égaux, deux appels doivent rendre la même liste : sinon le
    recyclage des lignes brasserait des souscriptions pour rien."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    un = ib_data.selection_vif(chaine, budget_lignes=20)
    deux = ib_data.selection_vif(chaine, budget_lignes=20)
    assert un.equals(deux)


# ---=== fusionner ===---

def _socle():
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    return chaine


def _vif(**champs):
    """Une ligne de vif sur le call 25 000, à l'échéance du socle."""
    ligne = {"ExpirationDate": EXP, "StrikePrice": 25_000.0, "right": "C"}
    ligne.update(champs)
    return pd.DataFrame([ligne])


def test_fusionner_rafraichit_l_iv_la_ou_le_vif_parle():
    """Ce qui bouge en séance, c'est le spot et l'IV."""
    fusionnee, spot = ib_data.fusionner(_socle(), _vif(IV=0.42), spot=25_100.0)
    ligne = fusionnee[fusionnee.StrikePrice == 25_000.0].iloc[0]
    assert ligne.CallIV == pytest.approx(0.42)
    assert spot == pytest.approx(25_100.0)


def test_fusionner_laisse_l_iv_du_socle_ailleurs():
    """Le vif ne couvre que ±2,5 % : les ailes gardent l'IV du socle."""
    fusionnee, _ = ib_data.fusionner(_socle(), _vif(IV=0.42), spot=PRIX)
    loin = fusionnee[fusionnee.StrikePrice == 24_500.0].iloc[0]
    assert loin.CallIV == pytest.approx(VOL_VRAI)


def test_fusionner_ne_touche_jamais_l_open_interest():
    """L'OI ne bouge pas en séance : la chambre de compensation le calcule après
    la clôture. Le figer n'est pas une approximation, c'est la seule valeur."""
    socle = _socle()
    fusionnee, _ = ib_data.fusionner(socle, _vif(IV=0.42, OpenInt=999_999.0),
                                     spot=PRIX)
    assert fusionnee.CallOpenInt.equals(socle.CallOpenInt)


def test_fusionner_distingue_les_calls_des_puts():
    """Même strike, même échéance : le sens doit trancher."""
    vif = _vif(IV=0.42)
    vif["right"] = "P"
    fusionnee, _ = ib_data.fusionner(_socle(), vif, spot=PRIX)
    ligne = fusionnee[fusionnee.StrikePrice == 25_000.0].iloc[0]
    assert ligne.PutIV == pytest.approx(0.42)
    assert ligne.CallIV == pytest.approx(VOL_VRAI)      # le call n'a pas bougé


def test_fusionner_rafraichit_aussi_le_gamma_publie():
    """--gamma-source published lirait sinon un gamma périmé."""
    fusionnee, _ = ib_data.fusionner(_socle(), _vif(IV=VOL_VRAI, Gamma=0.00123),
                                     spot=PRIX)
    ligne = fusionnee[fusionnee.StrikePrice == 25_000.0].iloc[0]
    assert ligne.CallGamma == pytest.approx(0.00123)


def test_fusionner_sans_vif_rend_le_socle_intact():
    """Au démarrage, ou après une déconnexion, le socle seul doit rester lisible."""
    socle = _socle()
    for vide in (None, pd.DataFrame(columns=ib_data.CONTRAT + ["IV"])):
        fusionnee, spot = ib_data.fusionner(socle, vide, spot=PRIX)
        assert fusionnee.equals(socle)
        assert spot == pytest.approx(PRIX)


def test_fusionner_alimente_analyser_sans_retouche():
    """Le but de toute la fonction : (df, spot) est l'entrée d'analyser()."""
    import analysis
    df, spot = ib_data.fusionner(_socle(), _vif(IV=0.42), spot=25_100.0)
    a = analysis.analyser(df, spot=spot, quote_date=QUOTE, ticker="NQ",
                          contract_size=20, dte_max=None)
    assert np.isfinite(a.total_gex)


def test_fusionner_ignore_un_contrat_absent_du_socle():
    """Le vif peut porter un strike que le socle n'avait pas : on ne l'invente pas."""
    socle = _socle()
    vif = _vif(IV=0.42)
    vif["StrikePrice"] = 99_999.0
    fusionnee, _ = ib_data.fusionner(socle, vif, spot=PRIX)
    assert len(fusionnee) == len(socle)
    assert 99_999.0 not in set(fusionnee.StrikePrice)


# ---=== branchement du lecteur ===---

def test_suivre_resout_le_chemin_du_courant(tmp_path):
    """--suivre NQ évite de taper snapshots/NQ/courant.parquet à la main."""
    import main
    import snapshots
    args = main.construire_parser().parse_args(["NQ", "--suivre", "--dir", str(tmp_path)])
    assert main.source_relecture(args) == snapshots.courant("NQ", str(tmp_path))


def test_sans_drapeau_la_source_reste_le_courant():
    """Il n'y a plus de téléchargement à faire : le collecteur écrit, le lecteur
    lit. Sans --replay, il ne reste que le courant, et l'exiger par un drapeau
    ferait un drapeau obligatoire donc inutile."""
    import main
    import snapshots
    args = main.construire_parser().parse_args(["NQ"])
    assert main.source_relecture(args) == snapshots.courant("NQ", snapshots.DOSSIER)


def test_watch_et_replay_restent_exclusifs_sur_une_archive(tmp_path):
    """Une archive horodatée ne bouge plus : la suivre n'a aucun sens."""
    import main
    args = main.construire_parser().parse_args(
        ["NQ", "--watch", "60", "--replay", str(tmp_path / "2026-08-25_1436.parquet")])
    with pytest.raises(ValueError, match="archive"):
        main.verifier_exclusions(args)


def test_watch_est_permis_sur_le_courant(tmp_path):
    """Le courant bouge : le suivre est exactement l'usage visé."""
    import main
    args = main.construire_parser().parse_args(
        ["NQ", "--watch", "60", "--suivre", "--dir", str(tmp_path)])
    main.verifier_exclusions(args)          # ne doit rien lever


# ---=== les champs que la souscription rend gratuitement ===---

def _trames_carnet():
    """Ce que le probe a vu arriver reellement : carnet, seance, grecs."""
    defs, ticks = _trames_ib()
    ticks = ticks.copy()
    ticks["Bid"] = 6.50
    ticks["Ask"] = 7.25
    ticks["BidSize"] = 71.0
    ticks["AskSize"] = 27.0
    ticks["Vol"] = 313.0
    ticks["LastSale"] = 10.0
    return defs, ticks


def test_build_chain_remplit_le_carnet_prevu_par_COLUMNS():
    """CallBid, CallAsk, CallVol, CallLastSale sont dans COLUMNS depuis toujours et
    sortaient vides. IB les sert dans la MEME souscription, sans ligne ni requete
    de plus : les jeter serait perdre une donnee gratuite."""
    chaine, _, _ = ib_data.build_chain(*_trames_carnet(), futures_price=PRIX,
                                       quote_date=QUOTE)
    for colonne in ("CallBid", "CallAsk", "CallVol", "CallLastSale",
                    "PutBid", "PutAsk", "PutVol", "PutLastSale"):
        assert chaine[colonne].notna().any(), f"{colonne} est vide"
    assert chaine.CallBid.iloc[0] == pytest.approx(6.50)
    assert chaine.CallVol.iloc[0] == pytest.approx(313.0)


def test_build_chain_garde_les_tailles_du_carnet():
    """Hors COLUMNS, donc a preserver explicitement du reindex. snapshots.py dit
    pourquoi : on archive le brut pour pouvoir mesurer autre chose plus tard."""
    chaine, _, _ = ib_data.build_chain(*_trames_carnet(), futures_price=PRIX,
                                       quote_date=QUOTE)
    for colonne in ("CallBidSize", "CallAskSize", "PutBidSize", "PutAskSize"):
        assert colonne in chaine.columns, f"{colonne} a ete jetee par le reindex"
    assert chaine.CallBidSize.iloc[0] == pytest.approx(71.0)
    assert chaine.CallAskSize.iloc[0] == pytest.approx(27.0)


def test_build_chain_sans_carnet_reste_valide():
    """Une source qui ne sert pas le carnet ne doit pas echouer pour autant."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert chaine.CallBid.isna().all()
    assert "CallBidSize" in chaine.columns
    assert chaine.CallOpenInt.sum() > 0          # le reste marche toujours


def test_le_carnet_traverse_un_aller_retour_snapshot(tmp_path):
    """Archiver puis relire doit conserver les tailles : sinon l'archive ne
    permet pas de reconstituer le flux a posteriori."""
    import snapshots
    chaine, prix, date_val = ib_data.build_chain(*_trames_carnet(),
                                                 futures_price=PRIX,
                                                 quote_date=QUOTE)
    chemin = snapshots.sauver(chaine, "NQ", prix, date_val, str(tmp_path))
    relu, _, _, _ = snapshots.charger(chemin)
    assert relu.CallBidSize.iloc[0] == pytest.approx(71.0)
    assert relu.CallVol.iloc[0] == pytest.approx(313.0)


def test_le_carnet_ne_derange_pas_analyser():
    """Des colonnes en plus ne doivent rien changer aux chiffres produits."""
    import analysis
    avec, prix, date_val = ib_data.build_chain(*_trames_carnet(),
                                               futures_price=PRIX, quote_date=QUOTE)
    sans, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                     quote_date=QUOTE)
    a = analysis.analyser(avec, spot=prix, quote_date=date_val, ticker="NQ",
                          contract_size=20, dte_max=None)
    b = analysis.analyser(sans, spot=prix, quote_date=date_val, ticker="NQ",
                          contract_size=20, dte_max=None)
    assert a.total_gex == pytest.approx(b.total_gex, rel=1e-12)


# ---=== lots ===---

def test_lots_ne_depasse_jamais_la_taille():
    """Saturer le quota de cent lignes fait échouer les souscriptions en silence."""
    paquets = ib_data.lots(_contrats_cotes(), taille=90)
    assert all(len(p) <= 90 for p in paquets)


def test_lots_ne_perd_ni_ne_duplique_aucun_contrat():
    """Un contrat oublié est un trou dans la chaîne, un contrat compté deux fois
    est une ligne payée deux fois."""
    contrats = _contrats_cotes()
    recolles = pd.concat(ib_data.lots(contrats, taille=90), ignore_index=True)
    assert len(recolles) == len(contrats)
    assert sorted(recolles.conId) == sorted(contrats.conId)


def test_lots_compte_juste():
    """Le nombre de lots est ce qui fixe le temps de balayage."""
    contrats = _contrats_cotes()
    n = len(contrats)
    assert len(ib_data.lots(contrats, taille=90)) == -(-n // 90)
    assert len(ib_data.lots(contrats, taille=n)) == 1
    assert len(ib_data.lots(contrats, taille=n + 1)) == 1


def test_lots_vide_rend_une_liste_vide():
    """Une échéance sans contrat dans la plage n'est pas une panne."""
    assert ib_data.lots(pd.DataFrame(columns=["conId", "StrikePrice"])) == []


def test_lots_refuse_une_taille_absurde():
    """Zéro contrat par lot bouclerait indéfiniment : échouer bruyamment."""
    with pytest.raises(ValueError):
        ib_data.lots(_contrats_cotes(), taille=0)


# ---=== ligne_ticker ===---

class _Grecs:
    """Imite ib_async.OptionComputation : seuls les champs lus comptent."""

    def __init__(self, impliedVol=None, delta=None, gamma=None, vega=None,
                 theta=None, undPrice=None):
        self.impliedVol, self.delta, self.gamma = impliedVol, delta, gamma
        self.vega, self.theta, self.undPrice = vega, theta, undPrice


class _Contrat:
    def __init__(self, conId, right, strike=29_050.0):
        self.conId, self.right, self.strike = conId, right, strike


class _Ticker:
    """Imite ib_async.Ticker, avec les valeurs relevées au sondage."""

    def __init__(self, contract, **champs):
        self.contract = contract
        defauts = dict(bid=float("nan"), ask=float("nan"), bidSize=float("nan"),
                       askSize=float("nan"), last=float("nan"),
                       lastSize=float("nan"), close=float("nan"),
                       volume=float("nan"), callOpenInterest=float("nan"),
                       putOpenInterest=float("nan"), modelGreeks=None)
        defauts.update(champs)
        for cle, valeur in defauts.items():
            setattr(self, cle, valeur)


def test_ligne_ticker_lit_un_call():
    """Valeurs relevées sur Q4BQ6 C29050 le 25 août 2026."""
    t = _Ticker(_Contrat(909426450, "C"),
                bid=199.5, ask=203.5, bidSize=2.0, askSize=2.0,
                last=207.0, close=134.5, volume=21.0,
                callOpenInterest=15.0, putOpenInterest=0.0,
                modelGreeks=_Grecs(impliedVol=0.2056, delta=0.8589,
                                   gamma=0.0015764, vega=1.5863, theta=-10.236,
                                   undPrice=29_206.85))
    ligne = ib_data.ligne_ticker(t)
    assert ligne["conId"] == 909426450
    assert ligne["OpenInt"] == pytest.approx(15.0)      # le CALL prend callOpenInterest
    assert ligne["Bid"] == pytest.approx(199.5)
    assert ligne["Ask"] == pytest.approx(203.5)
    assert ligne["BidSize"] == pytest.approx(2.0)
    assert ligne["Vol"] == pytest.approx(21.0)
    assert ligne["LastSale"] == pytest.approx(207.0)
    assert ligne["IV"] == pytest.approx(0.2056)
    assert ligne["Gamma"] == pytest.approx(0.0015764)
    assert ligne["Delta"] == pytest.approx(0.8589)
    assert ligne["UndPrice"] == pytest.approx(29_206.85)


def test_ligne_ticker_lit_un_put_du_bon_cote():
    """Valeurs relevées sur Q4BQ6 P29050 : l'OI est dans putOpenInterest."""
    t = _Ticker(_Contrat(909426451, "P"),
                callOpenInterest=0.0, putOpenInterest=39.0)
    assert ib_data.ligne_ticker(t)["OpenInt"] == pytest.approx(39.0)


def test_ligne_ticker_traduit_le_moins_un_en_absence():
    """IB code « pas de prix » par -1. Le laisser passer donnerait des primes
    négatives, et un implied_vol calculé sur du vide."""
    t = _Ticker(_Contrat(1, "C"), bid=-1.0, ask=-1.0, last=-1.0, close=-1.0)
    ligne = ib_data.ligne_ticker(t)
    for champ in ("Bid", "Ask", "LastSale", "Settle"):
        assert np.isnan(ligne[champ]), f"{champ} vaut {ligne[champ]}, attendu NaN"


def test_ligne_ticker_garde_une_taille_nulle():
    """Zéro au bid est une information — aucune quantité affichée — pas une
    absence de donnée. Contrairement au -1 des prix."""
    t = _Ticker(_Contrat(1, "C"), bidSize=0.0, askSize=0.0)
    ligne = ib_data.ligne_ticker(t)
    assert ligne["BidSize"] == pytest.approx(0.0)
    assert ligne["AskSize"] == pytest.approx(0.0)


def test_ligne_ticker_sans_grecs_ne_plante_pas():
    """Un contrat illiquide ne répond parfois jamais : modelGreeks reste None."""
    ligne = ib_data.ligne_ticker(_Ticker(_Contrat(1, "C"), callOpenInterest=7.0))
    assert ligne["OpenInt"] == pytest.approx(7.0)
    for champ in ("IV", "Gamma", "Delta", "Vega", "Theta", "UndPrice"):
        assert np.isnan(ligne[champ])


def test_ligne_ticker_prend_close_comme_settle():
    """Le règlement de la veille : ce dont infer_futures_price a besoin quand
    undPrice manque."""
    t = _Ticker(_Contrat(1, "C"), close=134.5)
    assert ib_data.ligne_ticker(t)["Settle"] == pytest.approx(134.5)


def test_ligne_ticker_rend_toutes_les_cles_attendues():
    """build_chain lit CHAMPS_TICK : une clé manquante ferait une colonne vide
    sans que rien ne le signale."""
    ligne = ib_data.ligne_ticker(_Ticker(_Contrat(1, "P")))
    for champ in ib_data.CHAMPS_TICK:
        assert champ in ligne, f"{champ} absent de la ligne"
    assert "conId" in ligne and "UndPrice" in ligne


def test_ligne_ticker_alimente_build_chain():
    """Le contrat de bout en bout : des Tickers doivent traverser build_chain, et
    l'undPrice y servir de prix du future."""
    tickers = []
    for i, (k, r) in enumerate([(29_000.0, "C"), (29_000.0, "P"),
                                (29_050.0, "C"), (29_050.0, "P")]):
        tickers.append(_Ticker(
            _Contrat(500_000 + i, r, strike=k),
            bid=10.0, ask=11.0, callOpenInterest=20.0, putOpenInterest=30.0,
            modelGreeks=_Grecs(impliedVol=0.20, delta=0.5, gamma=0.0015,
                               vega=1.5, theta=-10.0, undPrice=29_020.0)))
    ticks = pd.DataFrame([ib_data.ligne_ticker(t) for t in tickers])
    defs = pd.DataFrame([{"conId": t.contract.conId,
                          "StrikePrice": t.contract.strike,
                          "ExpirationDate": EXP,
                          "right": t.contract.right} for t in tickers])

    chaine, prix, _ = ib_data.build_chain(defs, ticks, futures_price=None,
                                          quote_date=QUOTE)
    assert len(chaine) == 2
    assert chaine.CallOpenInt.sum() == pytest.approx(40.0)
    assert chaine.PutOpenInt.sum() == pytest.approx(60.0)
    assert prix == pytest.approx(29_020.0)      # undPrice a servi de prix


# ---=== ib_collector : les decisions ===---

def test_rebalayage_une_fois_par_journee_de_compensation():
    """L'open interest ne bouge pas en séance : rebalayer plus souvent serait
    payer huit minutes pour relire le même chiffre."""
    import ib_collector as ic
    socle = pd.Timestamp("2026-08-25 20:00")          # avant 23 h UTC
    assert not ic.faut_il_rebalayer(socle, pd.Timestamp("2026-08-25 22:00"))
    assert ic.faut_il_rebalayer(socle, pd.Timestamp("2026-08-26 01:00"))


def test_rebalayage_si_aucun_socle():
    """Au démarrage il n'y a rien : il faut balayer."""
    import ib_collector as ic
    assert ic.faut_il_rebalayer(None, pd.Timestamp("2026-08-25 10:00"))


def test_journee_compensation_bascule_a_l_heure_du_cme():
    """Le CME publie l'open interest préliminaire à 18 h Chicago, soit 23 h UTC :
    après cette heure, on est déjà sur la publication du lendemain."""
    import ib_collector as ic
    avant = ic.journee_compensation(pd.Timestamp("2026-08-25 22:59"))
    apres = ic.journee_compensation(pd.Timestamp("2026-08-25 23:01"))
    assert apres > avant


def test_bande_couverte_encadre_le_vif():
    """La bande dit jusqu'où le vif voit, donc quand il cesse de voir."""
    import ib_collector as ic
    vif = pd.DataFrame({"StrikePrice": [28_900.0, 29_000.0, 29_100.0],
                        "ExpirationDate": [EXP] * 3, "right": ["C", "C", "P"]})
    assert ic.bande_couverte(vif) == (pytest.approx(28_900.0), pytest.approx(29_100.0))
    assert ic.bande_couverte(pd.DataFrame(columns=["StrikePrice"])) is None
    assert ic.bande_couverte(None) is None


def test_reselection_quand_le_spot_sort_de_la_bande():
    """Sans ça, les lignes entretenues finissent par regarder ailleurs que là où
    ça se passe."""
    import ib_collector as ic
    bande = (28_900.0, 29_100.0)
    assert not ic.faut_il_reselectionner(29_000.0, bande)
    assert ic.faut_il_reselectionner(29_500.0, bande)
    assert ic.faut_il_reselectionner(28_000.0, bande)


def test_reselection_avec_une_marge_avant_le_bord():
    """Attendre la sortie franche serait attendre d'être aveugle : on recycle
    quand le spot approche du bord."""
    import ib_collector as ic
    assert ic.faut_il_reselectionner(29_095.0, (28_900.0, 29_100.0), marge=0.10)
    assert not ic.faut_il_reselectionner(29_000.0, (28_900.0, 29_100.0), marge=0.10)


def test_reselection_sans_bande():
    """Aucun vif encore sélectionné : il en faut un."""
    import ib_collector as ic
    assert ic.faut_il_reselectionner(29_000.0, None)


# ---=== avec_conid ===---

def test_avec_conid_recolle_les_identifiants():
    """selection_vif travaille sur le format LARGE, où chaque ligne porte un call
    ET un put : les conId n'y survivent pas. Sans eux, rien n'est souscriptible."""
    catalogue = _contrats_cotes(strikes=[29_000.0, 29_100.0])
    vif = pd.DataFrame({"ExpirationDate": [pd.Timestamp("2026-09-04")] * 2,
                        "StrikePrice": [29_000.0, 29_100.0],
                        "right": ["C", "P"]})
    rendu = ib_data.avec_conid(vif, catalogue)
    assert "conId" in rendu.columns
    assert rendu.conId.notna().all()
    attendu_c = catalogue[(catalogue.StrikePrice == 29_000.0)
                          & (catalogue.right == "C")].conId.iloc[0]
    assert rendu.iloc[0].conId == attendu_c


def test_avec_conid_distingue_call_et_put_au_meme_strike():
    """Même échéance, même strike : seul le sens sépare les deux conId."""
    catalogue = _contrats_cotes(strikes=[29_000.0])
    vif = pd.DataFrame({"ExpirationDate": [pd.Timestamp("2026-09-04")] * 2,
                        "StrikePrice": [29_000.0, 29_000.0],
                        "right": ["C", "P"]})
    rendu = ib_data.avec_conid(vif, catalogue)
    assert rendu.conId.nunique() == 2


def test_avec_conid_ecarte_ce_qui_n_est_pas_au_catalogue():
    """Un contrat sans conId n'est pas souscriptible : le garder ferait planter
    la boucle au premier reqMktData."""
    catalogue = _contrats_cotes(strikes=[29_000.0])
    vif = pd.DataFrame({"ExpirationDate": [pd.Timestamp("2026-09-04")] * 2,
                        "StrikePrice": [29_000.0, 99_999.0],
                        "right": ["C", "C"]})
    rendu = ib_data.avec_conid(vif, catalogue)
    assert len(rendu) == 1
    assert rendu.iloc[0].StrikePrice == pytest.approx(29_000.0)


def test_avec_conid_preserve_l_ordre_de_selection():
    """L'ordre porte la priorité du vif : le recyclage doit garder les premiers."""
    catalogue = _contrats_cotes(strikes=[29_000.0, 29_100.0, 29_200.0])
    vif = pd.DataFrame({"ExpirationDate": [pd.Timestamp("2026-09-04")] * 3,
                        "StrikePrice": [29_200.0, 29_000.0, 29_100.0],
                        "right": ["C", "C", "C"]})
    rendu = ib_data.avec_conid(vif, catalogue)
    assert list(rendu.StrikePrice) == [29_200.0, 29_000.0, 29_100.0]


# ---=== l'heure de l'echeance, et celle du releve ===---

def test_instant_echeance_convertit_le_fuseau_de_la_place():
    """IB sert 15h00 US/Central pour une hebdomadaire NQ, soit 16h00 New York."""
    obtenu = ib_data.instant_echeance("20260826", "15:00:00", "US/Central")
    assert obtenu == pd.Timestamp("2026-08-26 16:00:00")


def test_instant_echeance_les_mensuelles_sont_reglees_le_matin():
    """C'est le piège que coder 16h en dur aurait manqué : la mensuelle NQ expire
    à 08h30 US/Central, soit 9h30 New York — six heures et demie plus tôt que
    l'hebdomadaire, ce qui le jour de l'échéance sépare le vivant du mort."""
    obtenu = ib_data.instant_echeance("20260918", "08:30:00", "US/Central")
    assert obtenu == pd.Timestamp("2026-09-18 09:30:00")


def test_instant_echeance_sans_heure_suppose_la_cloture():
    """IB cesse de servir l'heure une fois l'échéance passée. Le majorant fait
    survivre le contrat quelques heures de trop plutôt que de le tuer trop tôt."""
    assert ib_data.instant_echeance("20260825") == pd.Timestamp("2026-08-25 16:00:00")
    assert ib_data.instant_echeance("20260825", "", "") == pd.Timestamp("2026-08-25 16:00:00")


def test_instant_echeance_refuse_un_fuseau_inconnu():
    """Le traiter comme New York décalerait l'échéance d'une heure ronde sans que
    rien ne le signale."""
    with pytest.raises(Exception):
        ib_data.instant_echeance("20260826", "15:00:00", "Mars/Olympus_Mons")


def test_quote_date_garde_l_heure_de_collecte():
    """La régression que ce test ferme : .normalize() écrasait l'heure, ce qui
    donnait au 0DTE le décalage EDT/UTC pour temps restant et empêchait
    main.suivre() de jamais voir un relevé avancer."""
    assert ib_data._quote_date("2026-08-25 20:04:37") == pd.Timestamp("2026-08-25 20:04:37")


def test_quote_date_ramene_a_utc():
    """L'échéance est en heure de New York, le relevé en UTC : c'est le contrat
    qu'applique time_to_expiry, et les archives le suivent."""
    assert (ib_data._quote_date(pd.Timestamp("2026-08-25 16:04:00", tz="America/New_York"))
            == pd.Timestamp("2026-08-25 20:04:00"))


def test_echeances_utiles_compte_des_jours_pas_des_heures():
    """Avec l'heure de collecte, une échéance du jour rendrait -1 en soustrayant
    les instants, et serait écartée alors qu'elle est précisément le 0DTE."""
    retenues = ib_data.echeances_utiles(
        ["20260825", "20260826"], quote_date="2026-08-25 20:04:00", dte_max=1, dte_min=0)
    assert [d.date().isoformat() for d in retenues] == ["2026-08-25", "2026-08-26"]
