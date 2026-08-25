"""Le pipeline d'analyse : filtre d'échéance, expositions, murs, profil, zero gamma.

Cette logique vivait dans main(), donc rien ne la testait — les 88 tests
précédents couvraient les formules et les parsers, et pas un seul des chiffres
réellement affichés. Elle est maintenant dans analysis.py, appelable sans réseau.

Les chaînes d'essai sont construites depuis des paramètres connus (spot, IV, OI
placés à des strikes choisis), ce qui permet de vérifier qu'on retrouve la
réponse attendue plutôt que de figer une sortie.
"""

import shutil

import numpy as np
import pandas as pd
import pytest

import analysis
import chain
import greeks as G
import plots
import snapshots
from chain import COLUMNS

SPOT = 100.0
QUOTE = pd.Timestamp("2026-08-12")
IV = 0.25


def chaine(spot=SPOT, quote_date=QUOTE, jours=(30,), strikes=None,
           call_oi=None, put_oi=None, iv=IV, facteur_gamma_publie=1.0, skew=0.0):
    """Chaîne synthétique au format COLUMNS, avec un gamma publié cohérent avec l'IV.

    L'open interest par défaut est volontairement ASYMÉTRIQUE entre calls et puts :
    à OI égal, le GEX net, le charm et la vanna sont identiquement nuls (les deux
    jambes se compensent exactement), et tous les tests passeraient sur du vide.

    `facteur_gamma_publie` permet de désaccorder volontairement le gamma diffusé
    par la source et celui recalculé : c'est exactement la situation réelle des
    échéances courtes, où l'écart mesuré atteint 29 % sur le SPX.
    """
    strikes = np.arange(70.0, 131.0, 5.0) if strikes is None else np.asarray(strikes, float)
    lignes = []
    for j in jours:
        echeance = pd.Timestamp(quote_date) + pd.Timedelta(days=int(j)) + pd.Timedelta(hours=16)
        for k in strikes:
            lignes.append({"ExpirationDate": echeance, "StrikePrice": k})
    df = pd.DataFrame(lignes)

    n = len(strikes)
    defaut_call = np.full(n, 120.0)
    defaut_put = np.full(n, 80.0)
    df["CallOpenInt"] = np.tile(defaut_call if call_oi is None else np.asarray(call_oi, float),
                                len(jours))
    df["PutOpenInt"] = np.tile(defaut_put if put_oi is None else np.asarray(put_oi, float),
                               len(jours))
    # skew : pente dIV/d(ln K/S). Négative sur actions et indices — les puts
    # hors de la monnaie se paient plus cher que les calls.
    smile = iv + skew * np.log(df.StrikePrice.values / spot)
    df["CallIV"] = np.maximum(smile, 0.01)
    df["PutIV"] = np.maximum(smile, 0.01)

    # Gamma unitaire vrai, celui qu'une source honnête publierait
    T, _ = analysis.time_to_expiry(df.ExpirationDate, quote_date)
    unitaire = G.calc_gamma_ex(spot, df.StrikePrice, df.CallIV, T, 0, 0, "call",
                               OI=1, contract_size=1) / (spot ** 2 * 0.01)
    df["CallGamma"] = unitaire * facteur_gamma_publie
    df["PutGamma"] = unitaire * facteur_gamma_publie
    # Delta et vega comme le CBOE les publierait : delta de call croissant avec
    # la monnaie, vega en cloche autour d'elle. DEX et VEX en dépendent, et une
    # chaîne d'essai qui les laisserait à zéro ne testerait rien.
    ecart = (df.StrikePrice.values - spot) / spot
    df["CallDelta"] = np.clip(0.5 - ecart * 4, 0.01, 0.99)
    df["PutDelta"] = df["CallDelta"] - 1.0
    df["CallVega"] = np.exp(-((ecart / 0.15) ** 2)) * 0.2
    df["PutVega"] = df["CallVega"]

    for colonne in COLUMNS + chain.COLONNES_GRECS:
        if colonne not in df.columns:
            df[colonne] = 0.0
    return df[COLUMNS + chain.COLONNES_GRECS]


# ---=== Filtre d'échéance ===---

def test_filtre_ne_garde_que_la_fenetre_demandee():
    df = chaine(jours=(0, 3, 10, 45, 400))
    garde = analysis.filtre_echeances(df, QUOTE, dte_max=30, dte_min=0)
    restants = sorted(analysis.dte_calendaire(garde, QUOTE).unique())
    assert restants == [0, 3, 10]


def test_filtre_dte_min_exclut_les_echeances_les_plus_proches():
    """--dte-min 2 est la parade documentée à l'instabilité des 0-1 DTE."""
    garde = analysis.filtre_echeances(chaine(jours=(0, 1, 5)), QUOTE, dte_max=30, dte_min=2)
    assert sorted(analysis.dte_calendaire(garde, QUOTE).unique()) == [5]


def test_filtre_all_garde_tout():
    df = chaine(jours=(1, 400))
    assert len(analysis.filtre_echeances(df, QUOTE, dte_max=None)) == len(df)


def test_filtre_vide_dit_quelle_valeur_essayer():
    """Le message doit donner l'échéance la plus proche, pas seulement échouer."""
    with pytest.raises(ValueError, match="45"):
        analysis.filtre_echeances(chaine(jours=(45, 90)), QUOTE, dte_max=30)


def test_filtre_sans_echeance_future():
    with pytest.raises(ValueError, match="aucune échéance future"):
        analysis.filtre_echeances(chaine(jours=(-10,)), QUOTE, dte_max=30)


# ---=== Convention de temps ===---

def test_time_to_expiry_heures_compte_le_temps_reel_restant():
    """Un 0DTE à 10h du matin, c'est ~0,25 jour, pas 1."""
    asof = pd.Timestamp("2026-08-12 14:00")              # 10h00 New York (EDT)
    exp = pd.Series([pd.Timestamp("2026-08-12 16:00")])  # échéance 16h NY le jour même
    T, div = analysis.time_to_expiry(exp, asof, "heures")
    assert div == 365.0
    assert T[0] * 365 == pytest.approx(6 / 24, abs=0.02)   # 6 heures restantes


def test_time_to_expiry_bourse_applique_le_plancher_d_un_jour():
    asof = pd.Timestamp("2026-08-12 14:00")
    exp = pd.Series([pd.Timestamp("2026-08-12 16:00")])
    T, div = analysis.time_to_expiry(exp, asof, "bourse")
    assert div == G.TRADING_DAYS
    assert T[0] == pytest.approx(1 / G.TRADING_DAYS)       # plancher, pas 0,25 jour


def test_le_charm_suit_le_diviseur_de_la_convention_active():
    """Une dérivée temporelle divisée par le mauvais diviseur est fausse d'un facteur 365/262.

    time_to_expiry renvoie T et son diviseur ensemble précisément pour ça ; le
    test vérifie que analyser() ne les désaccorde pas en chemin.
    """
    df = chaine(jours=(10,))
    heures = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, time_convention="heures")
    bourse = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, time_convention="bourse")

    T_h, div_h = analysis.time_to_expiry(heures.df.ExpirationDate, QUOTE, "heures")
    attendu = G.calc_charm_ex(SPOT, heures.df.StrikePrice, heures.df.CallIV, T_h,
                              heures.df.CallOpenInt, 100, jours_par_an=div_h)
    reel = G.calc_charm_ex(SPOT, heures.df.StrikePrice, heures.df.CallIV, T_h,
                           heures.df.CallOpenInt, 100)      # défaut : 262, faux ici
    assert heures.df.CallCharm.values == pytest.approx(attendu, rel=1e-9)
    assert not np.allclose(attendu, reel)
    assert heures.total_charm != pytest.approx(bourse.total_charm, rel=1e-6)


def test_la_convention_de_temps_deplace_le_zero_gamma():
    """Sur le SPX réel l'écart est de 12 points : ce n'est pas un détail cosmétique."""
    df = chaine(jours=(0, 1, 20))
    heures = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, time_convention="heures")
    bourse = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, time_convention="bourse")
    assert heures.time_convention == "heures" and bourse.time_convention == "bourse"
    assert heures.total_gex != pytest.approx(bourse.total_gex, rel=1e-6)


# ---=== Source de gamma ===---

def test_les_deux_sources_coincident_quand_la_source_est_honnete():
    df = chaine(jours=(7,))
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    exp = analysis.expositions(df, SPOT, T, 100, source="iv")
    assert exp.GEXiv.sum() == pytest.approx(exp.GEXpublie.sum(), rel=1e-9)
    autre = analysis.expositions(df, SPOT, T, 100, source="published")
    assert autre.TotalGamma.sum() == pytest.approx(exp.TotalGamma.sum(), rel=1e-9)


def test_la_source_choisie_est_bien_celle_utilisee():
    """Le vrai régression test : un gamma publié faux ne doit contaminer que "published"."""
    df = chaine(jours=(1,), facteur_gamma_publie=1.30)
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    iv = analysis.expositions(df, SPOT, T, 100, source="iv")
    pub = analysis.expositions(df, SPOT, T, 100, source="published")
    assert pub.TotalGamma.sum() == pytest.approx(1.30 * iv.TotalGamma.sum(), rel=1e-9)
    # et l'écart est mesuré, pas seulement subi
    _, _, relatif = analysis.ecart_gamma(iv)
    assert relatif == pytest.approx(1 / 1.30 - 1, rel=1e-6)


def test_source_de_gamma_inconnue_refusee():
    df = chaine()
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    with pytest.raises(ValueError, match="source de gamma"):
        analysis.expositions(df, SPOT, T, 100, source="bloomberg")


def test_le_total_gex_egale_le_profil_au_spot():
    """LE point du chantier : les deux chiffres affichés viennent du même estimateur.

    Avant, le Total GEX prenait le gamma publié pendant que le zero gamma
    recalculait depuis l'IV. Ils ne pouvaient donc pas coïncider — 13 % d'écart
    mesuré sur le SPX à 30 jours, 29 % à un jour.
    """
    a = analysis.analyser(chaine(jours=(1, 8, 25), facteur_gamma_publie=1.30),
                          spot=SPOT, quote_date=QUOTE, source_gamma="iv")
    au_spot = analysis.profil_gamma(a.df, [SPOT], a.contract_size).sum()
    assert a.total_gex == pytest.approx(au_spot, rel=1e-9)


# ---=== Murs ===---

def _chaine_avec_murs():
    """Concentrations placées exprès : la bande gamma (+/-15 %) voit 110 et 90,
    la bande OI (+/-30 %) voit 120 et 75.

    L'OI concentré doit être franchement plus gros que la base, sinon le strike
    ATM gagne quand même : c'est le phénomène décrit dans le README, le gamma
    unitaire y étant maximal au point de battre des strikes bien plus chargés.
    """
    strikes = np.arange(70.0, 131.0, 5.0)
    call_oi = np.full(len(strikes), 120.0)
    put_oi = np.full(len(strikes), 80.0)
    call_oi[strikes == 110] = 5_000       # dans la bande gamma
    call_oi[strikes == 120] = 20_000      # hors bande gamma, dans la bande OI
    put_oi[strikes == 90] = 5_000
    put_oi[strikes == 75] = 20_000
    return chaine(strikes=strikes, call_oi=call_oi, put_oi=put_oi)


def test_les_murs_gamma_encadrent_le_spot():
    a = analysis.analyser(_chaine_avec_murs(), spot=SPOT, quote_date=QUOTE)
    assert a.call_wall == 110 and a.put_wall == 90
    assert a.call_wall > SPOT > a.put_wall


def test_les_murs_en_oi_voient_plus_loin_que_les_murs_gamma():
    """Les murs pondérés par le gamma restent collés à la monnaie ; c'est l'écart
    entre les deux lectures qui porte l'information sur un indice."""
    a = analysis.analyser(_chaine_avec_murs(), spot=SPOT, quote_date=QUOTE)
    assert a.call_wall_oi == 120 and a.put_wall_oi == 75


def test_sans_gamma_call_il_n_y_a_pas_de_mur():
    """idxmax sur une colonne nulle renverrait le premier strike de la bande."""
    strikes = np.arange(70.0, 131.0, 5.0)
    a = analysis.analyser(chaine(strikes=strikes, call_oi=np.zeros(len(strikes))),
                          spot=SPOT, quote_date=QUOTE)
    assert a.call_wall is None and a.call_wall_oi is None
    assert a.put_wall is not None


def test_le_mur_gamma_ne_paraphrase_pas_le_spot():
    """Sans la contrainte de côté, les deux murs tombent sur le strike ATM."""
    a = analysis.analyser(_chaine_avec_murs(), spot=SPOT, quote_date=QUOTE)
    assert a.call_wall != a.put_wall


# ---=== Profil et zero gamma ===---

def test_profil_vectorise_egale_la_boucle():
    df = chaine(jours=(3, 20))
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    df = analysis.expositions(df, SPOT, T, 100)
    niveaux = np.linspace(80, 120, 7)
    matrice = analysis.profil_gamma(df, niveaux, 100)
    for i, niveau in enumerate(niveaux):
        attendu = (G.calc_gamma_ex(niveau, df.StrikePrice, df.CallIV, df["T"], 0, 0,
                                   "call", df.CallOpenInt, 100)
                   - G.calc_gamma_ex(niveau, df.StrikePrice, df.PutIV, df["T"], 0, 0,
                                     "put", df.PutOpenInt, 100))
        assert matrice[i] == pytest.approx(attendu, rel=1e-12)


# ---=== Régime de volatilité ===---

def test_pente_skew_retrouve_la_pente_injectee():
    """Le smile est construit avec une pente connue : on doit la retrouver."""
    df = chaine(jours=(30,), skew=-0.40)
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    df = analysis.expositions(df, SPOT, T, 100)
    assert analysis.pente_skew(df, SPOT)[0] == pytest.approx(-0.40, rel=1e-6)


def test_sans_skew_les_deux_regimes_coincident():
    """Un smile plat n'a rien à décaler : les deux profils doivent être identiques."""
    df = chaine(jours=(30,), skew=0.0)
    a = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, regime_vol="sticky-strike")
    b = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, regime_vol="sticky-moneyness")
    assert b.profiles["All Expiries"] == pytest.approx(a.profiles["All Expiries"], rel=1e-9)
    assert b.zero_gamma == pytest.approx(a.zero_gamma, rel=1e-9)


def _livre_qui_croise(skew=0.0):
    """Puts chargés en dessous, calls au-dessus : le profil traverse zéro au milieu."""
    strikes = np.arange(70.0, 131.0, 5.0)
    return chaine(strikes=strikes, jours=(30,), skew=skew,
                  put_oi=np.where(strikes < SPOT, 5_000.0, 100.0),
                  call_oi=np.where(strikes > SPOT, 5_000.0, 100.0))


def test_le_decalage_de_vol_est_nul_au_spot():
    """Propriété exacte du régime : ln(S0/S') = 0 quand S' = S0.

    C'est ce qui borne sa portée. Le régime remodèle les ailes du profil sans
    toucher au voisinage du spot — donc il ne corrige PAS un zero gamma proche
    du spot, contrairement à ce qu'on pourrait attendre d'un « profil plus
    réaliste ».
    """
    df = _livre_qui_croise(skew=-0.60)
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    df = analysis.expositions(df, SPOT, T, 100)
    fige = analysis.profil_gamma(df, [SPOT], 100, SPOT, "sticky-strike")
    suivi = analysis.profil_gamma(df, [SPOT], 100, SPOT, "sticky-moneyness")
    assert suivi == pytest.approx(fige, rel=1e-12)


def test_sticky_moneyness_remodele_les_ailes():
    """Loin du spot, le décalage mord — et d'autant plus que le skew est marqué."""
    df = _livre_qui_croise(skew=-0.60)
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    df = analysis.expositions(df, SPOT, T, 100)
    niveaux = np.array([80.0, SPOT, 120.0])
    fige = analysis.profil_gamma(df, niveaux, 100, SPOT, "sticky-strike").sum(axis=1)
    suivi = analysis.profil_gamma(df, niveaux, 100, SPOT, "sticky-moneyness").sum(axis=1)

    assert suivi[1] == pytest.approx(fige[1], rel=1e-12)      # au spot, rien ne bouge
    assert not np.isclose(suivi[0], fige[0], rtol=1e-3)       # aile gauche
    assert not np.isclose(suivi[2], fige[2], rtol=1e-3)       # aile droite


def test_le_regime_de_vol_ne_touche_pas_au_gex_au_spot():
    """Le GEX affiché est mesuré au spot : aucun régime de profil ne doit le changer."""
    df = _livre_qui_croise(skew=-0.60)
    fige = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, regime_vol="sticky-strike")
    suivi = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, regime_vol="sticky-moneyness")
    assert suivi.total_gex == pytest.approx(fige.total_gex, rel=1e-12)
    assert suivi.call_wall == fige.call_wall and suivi.put_wall == fige.put_wall


def test_regime_de_volatilite_inconnu_refuse():
    df = chaine()
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    df = analysis.expositions(df, SPOT, T, 100)
    with pytest.raises(ValueError, match="régime de volatilité"):
        analysis.profil_gamma(df, [SPOT], 100, SPOT, "sticky-tout-ce-qu-on-veut")


def test_sticky_moneyness_sans_spot_retombe_sur_sticky_strike():
    """Sans spot de référence, aucun décalage n'est défini : pas d'invention."""
    df = chaine(skew=-0.60)
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    df = analysis.expositions(df, SPOT, T, 100)
    niveaux = np.linspace(80, 120, 9)
    sans = analysis.profil_gamma(df, niveaux, 100, None, "sticky-moneyness")
    fige = analysis.profil_gamma(df, niveaux, 100, SPOT, "sticky-strike")
    assert sans == pytest.approx(fige, rel=1e-12)


def test_dex_et_vex_suivent_la_convention_de_signe():
    """Dealers longs les calls, shorts les puts — comme pour le GEX.

    Le delta d'un put étant négatif, en être short AJOUTE du delta positif : les
    deux jambes se cumulent au lieu de se compenser, et un signe inversé ici
    donnerait un DEX proche de zéro sur un book pourtant très directionnel.
    """
    df = chaine(jours=(30,))
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    exp = analysis.expositions(df, SPOT, T, 100)

    attendu = ((df.CallDelta * df.CallOpenInt - df.PutDelta * df.PutOpenInt) * 100 * SPOT)
    assert exp.TotalDelta.values == pytest.approx(attendu.values, rel=1e-12)
    assert exp.TotalDelta.sum() > 0
    assert exp.TotalVega.sum() != 0


def test_une_source_sans_grecs_donne_une_exposition_nulle():
    """Le CME ne publie ni delta ni vega : zéro, pas une erreur d'attribut."""
    df = chaine(jours=(30,)).drop(columns=["CallDelta", "PutDelta", "CallVega", "PutVega"])
    T, _ = analysis.time_to_expiry(df.ExpirationDate, QUOTE)
    exp = analysis.expositions(df, SPOT, T, 100)
    assert (exp.TotalDelta == 0).all() and (exp.TotalVega == 0).all()
    assert exp.TotalGamma.sum() != 0        # le gamma, lui, reste calculé


def test_le_gex_en_titres_est_le_meme_chiffre_dans_une_autre_unite():
    """titres par $ = GEX($ / 1 %) / (0,01 x S^2). Deux lectures, une donnée."""
    a = analysis.analyser(chaine(), spot=SPOT, quote_date=QUOTE)
    titres = a.par_strike.TotalGammaTitres
    assert titres.values == pytest.approx(
        (a.par_strike.TotalGamma / (0.01 * SPOT ** 2)).values, rel=1e-12)


def test_le_smile_ignore_les_iv_nulles():
    """Les contrats très dans la monnaie sortent avec une IV de 0 chez le CBOE.

    Les moyenner tirerait le smile vers zéro et inventerait un sourire qui
    n'existe pas ; ils doivent être absents, pas comptés.
    """
    df = chaine(jours=(30,), skew=-0.4)
    df.loc[df.StrikePrice < 85, "CallIV"] = 0.0
    a = analysis.analyser(df, spot=SPOT, quote_date=QUOTE)
    bas = a.par_strike.CallIV[a.par_strike.index < 85]
    assert bas.isna().all()
    assert a.par_strike.CallIV[a.par_strike.index >= 85].notna().any()


def test_zero_gamma_retient_le_croisement_le_plus_proche_du_spot():
    """L'ancien code prenait le premier croisement de la fenêtre, donc le plus bas.

    Ici l'aile gauche est bruyante et repasse par zéro à 91,5, alors que la vraie
    bascule de régime est à 104,5 — juste au-dessus d'un spot à 100. L'ancienne
    version annonçait 91,5, soit 8,5 % en dessous du spot.
    """
    niveaux = np.linspace(90, 110, 21)
    profil = np.array([1, 1] + [-1] * 13 + [1, 2, 3, 4, 5, 6], dtype=float)
    assert len(analysis.croisements_zero(niveaux, profil)) == 2
    assert analysis.find_zero_gamma(niveaux, profil, spot=100.0) == pytest.approx(104.5)
    assert analysis.find_zero_gamma(niveaux, profil) == pytest.approx(91.5)   # ancien choix


def test_zero_gamma_interpole_le_changement_de_signe():
    niveaux = np.array([90.0, 100.0, 110.0])
    profil = np.array([-10.0, -5.0, 5.0])   # croise zéro entre 100 et 110
    assert analysis.find_zero_gamma(niveaux, profil, 100.0) == pytest.approx(105.0)


def test_zero_gamma_sans_croisement_renvoie_none():
    assert analysis.find_zero_gamma(np.array([1.0, 2.0]), np.array([3.0, 4.0])) is None
    assert analysis.croisements_zero(np.array([1.0, 2.0]), np.array([3.0, 4.0])) == []


def test_un_book_charge_en_puts_bascule_sous_le_spot():
    """Contrôle de bout en bout : beaucoup de puts en dessous -> gamma négatif en bas."""
    strikes = np.arange(70.0, 131.0, 5.0)
    put_oi = np.where(strikes < SPOT, 5_000.0, 100.0)
    a = analysis.analyser(chaine(strikes=strikes, put_oi=put_oi, jours=(20,)),
                          spot=SPOT, quote_date=QUOTE)
    assert a.zero_gamma is not None
    profil = a.profiles["All Expiries"]
    assert profil[0] < 0 < profil[-1]


def test_pick_scale_bascule_au_milliard():
    assert analysis.pick_scale([5e8])[1] == "millions"
    assert analysis.pick_scale([2e9])[1] == "milliards"
    assert analysis.pick_scale([])[1] == "millions"
    assert analysis.pick_scale([None, np.nan, 3e9])[1] == "milliards"


# ---=== Assemblage ===---

def test_analyser_expose_le_perimetre_et_la_source():
    a = analysis.analyser(chaine(jours=(1, 40)), spot=SPOT, quote_date=QUOTE,
                          ticker="TEST", dte_max=30, source_gamma="published")
    assert a.horizon == "<= 30j" and a.source_gamma == "published"
    assert a.df.ExpirationDate.nunique() == 1        # l'échéance à 40 jours est sortie
    assert a.from_strike < SPOT < a.to_strike


def test_le_profil_ex_next_expiry_differe_du_profil_complet():
    a = analysis.analyser(chaine(jours=(1, 8, 25)), spot=SPOT, quote_date=QUOTE)
    complet = a.profiles["All Expiries"]
    sans = a.profiles["Ex-Next Expiry"]
    assert not np.allclose(complet, sans)


def test_poids_des_echeances_courtes_mesure():
    """Le poids des 0-1 DTE est mesuré sur le GEX ET sur le charm.

    Lequel des deux domine dépend de la chaîne réelle (sur le SPX c'est le charm,
    qui varie en 1/T) : ça se constate, ça ne se garantit pas. Ce qu'on vérifie
    ici, c'est que les deux sont bien mesurés, sur le périmètre filtré.
    """
    a = analysis.analyser(chaine(jours=(1, 25)), spot=SPOT, quote_date=QUOTE)
    assert set(a.part_courtes) == {"du GEX", "du charm"}
    assert all(0 < p <= 1 for p in a.part_courtes.values())


def test_sans_echeance_courte_aucun_avertissement():
    a = analysis.analyser(chaine(jours=(12, 25)), spot=SPOT, quote_date=QUOTE)
    assert a.part_courtes == {}


def test_decimales_suivent_le_sous_jacent():
    """Une paire FX se lit en pips, un indice en points."""
    fx = analysis.analyser(chaine(spot=1.065, strikes=np.arange(0.9, 1.25, 0.025)),
                           spot=1.065, quote_date=QUOTE)
    assert fx.decimals == 4
    assert analysis.analyser(chaine(), spot=SPOT, quote_date=QUOTE).decimals == 2


# ---=== Archivage ===---

def test_snapshot_aller_retour(tmp_path):
    """Rejouer une séance archivée doit redonner exactement la même analyse."""
    df = chaine(jours=(1, 8, 25))
    chemin = snapshots.sauver(df, "TEST", SPOT, QUOTE, str(tmp_path))
    relu, spot, quote_date, marche = snapshots.charger(chemin)

    assert spot == SPOT and pd.Timestamp(quote_date) == QUOTE
    assert marche == {}                     # rien n'a été archivé, rien n'est inventé
    avant = analysis.analyser(df, spot=SPOT, quote_date=QUOTE)
    apres = analysis.analyser(relu, spot=spot, quote_date=quote_date)
    assert apres.total_gex == pytest.approx(avant.total_gex, rel=1e-9)
    assert apres.zero_gamma == pytest.approx(avant.zero_gamma, rel=1e-9)


def test_snapshot_transporte_le_contexte_de_marche(tmp_path):
    """Sans le contexte archivé, une séance rejouée perd le ratio au volume."""
    marche = {"open": 135.0, "high": 146.1, "low": 134.0, "close": 144.9,
              "volume": 95_310_234.0, "dollar_volume": 1.3808e10, "iv30": 70.2}
    chemin = snapshots.sauver(chaine(), "TEST", SPOT, QUOTE, str(tmp_path), marche)
    _, _, _, relu = snapshots.charger(chemin)
    assert relu == pytest.approx(marche)


def test_gex_sur_volume_rapporte_le_flux_a_ce_qui_s_echange():
    """120 M$ de couverture face à 13,8 Md$ échangés : un frottement, pas le moteur."""
    a = analysis.analyser(chaine(), spot=SPOT, quote_date=QUOTE,
                          marche={"dollar_volume": 1.3808e10})
    assert a.gex_sur_volume == pytest.approx(abs(a.total_gex) / 1.3808e10)
    assert analysis.analyser(chaine(), spot=SPOT, quote_date=QUOTE).gex_sur_volume is None


def test_snapshot_permet_de_changer_d_horizon_apres_coup():
    """C'est la raison d'être de l'archive : history.csv ne garde que le dérivé."""
    df = chaine(jours=(1, 25))
    court = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, dte_max=7)
    long = analysis.analyser(df, spot=SPOT, quote_date=QUOTE, dte_max=30)
    assert court.df.ExpirationDate.nunique() == 1
    assert long.df.ExpirationDate.nunique() == 2
    assert court.total_gex != long.total_gex


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
    """Le courant n'est pas une archive : il ne doit pas remonter dans --replay."""
    archive = snapshots.sauver(chaine(), "NQ", SPOT, QUOTE, str(tmp_path))
    vivant = snapshots.courant("NQ", str(tmp_path))
    shutil.copy(archive, vivant)            # le courant existe sur disque

    trouves = snapshots.lister("NQ", str(tmp_path))
    assert vivant not in trouves
    assert archive in trouves
    assert snapshots.dernier("NQ", str(tmp_path)) != vivant


def test_snapshot_courant_relu_comme_une_archive(tmp_path):
    """Même format que les archives : main.py --replay doit pouvoir l'ouvrir."""
    df = chaine(jours=(1, 8))
    archive = snapshots.sauver(df, "NQ", SPOT, QUOTE, str(tmp_path))
    cible = snapshots.courant("NQ", str(tmp_path))
    shutil.copy(archive, cible)

    relu, spot, quote_date, marche = snapshots.charger(cible)
    assert spot == SPOT and pd.Timestamp(quote_date) == QUOTE
    assert len(relu) == len(df)


def test_charger_un_fichier_qui_n_est_pas_un_releve(tmp_path):
    faux = tmp_path / "faux.csv.gz"
    pd.DataFrame({"a": [1]}).to_csv(faux, index=False, compression="gzip")
    with pytest.raises(ValueError, match="relevé archivé"):
        snapshots.charger(str(faux))


def test_charger_un_fichier_absent(tmp_path):
    with pytest.raises(FileNotFoundError):
        snapshots.charger(str(tmp_path / "absent.parquet"))


# ---=== Graphiques ===---

def test_les_quatre_graphiques_sont_ecrits(tmp_path):
    a = analysis.analyser(_chaine_avec_murs(), spot=SPOT, quote_date=QUOTE, ticker="TEST")
    chemins = plots.tracer(a, str(tmp_path))
    plots.fermer()
    assert len(chemins) == 4
    assert all(p.endswith(".png") for p in chemins)
    import os
    assert all(os.path.getsize(p) > 0 for p in chemins)
