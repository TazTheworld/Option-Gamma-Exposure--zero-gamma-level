"""Transactions d'initiés : filtrage du flux et lecture des formulaires 4.

Tout est testé sur des documents construits ici, jamais sur le réseau. Les deux
pièges réels du format sont reproduits : le flux EDGAR mélange les types de
formulaires, et un même dépôt y apparaît deux fois.
"""

import pandas as pd
import pytest

import sec_data

# Un formulaire 4 minimal mais fidèle : un achat sur le marché et une attribution.
FORMULAIRE = """<?xml version="1.0"?>
<ownershipDocument>
  <documentType>4</documentType>
  <periodOfReport>2026-08-11</periodOfReport>
  <issuer>
    <issuerCik>0000320193</issuerCik>
    <issuerName>Exemple Corp</issuerName>
    <issuerTradingSymbol>xmpl</issuerTradingSymbol>
  </issuer>
  <reportingOwner>
    <reportingOwnerId>
      <rptOwnerCik>0001111111</rptOwnerCik>
      <rptOwnerName>Martin Claire</rptOwnerName>
    </reportingOwnerId>
    <reportingOwnerAddress>
      <rptOwnerCity>Cupertino</rptOwnerCity><rptOwnerState>CA</rptOwnerState>
    </reportingOwnerAddress>
    <reportingOwnerRelationship>
      <isDirector>0</isDirector><isOfficer>1</isOfficer>
      <isTenPercentOwner>0</isTenPercentOwner><isOther>0</isOther>
      <officerTitle>Directrice financière</officerTitle>
    </reportingOwnerRelationship>
  </reportingOwner>
  <nonDerivativeTable>
    <nonDerivativeTransaction>
      <securityTitle><value>Action ordinaire</value></securityTitle>
      <transactionDate><value>2026-08-11</value></transactionDate>
      <transactionCoding><transactionCode>P</transactionCode></transactionCoding>
      <transactionAmounts>
        <transactionShares><value>1500</value></transactionShares>
        <transactionPricePerShare><value>42.50</value></transactionPricePerShare>
        <transactionAcquiredDisposedCode><value>A</value></transactionAcquiredDisposedCode>
      </transactionAmounts>
      <postTransactionAmounts>
        <sharesOwnedFollowingTransaction><value>18200</value></sharesOwnedFollowingTransaction>
      </postTransactionAmounts>
    </nonDerivativeTransaction>
    <nonDerivativeTransaction>
      <securityTitle><value>Action ordinaire</value></securityTitle>
      <transactionDate><value>2026-08-11</value></transactionDate>
      <transactionCoding><transactionCode>A</transactionCode></transactionCoding>
      <transactionAmounts>
        <transactionShares><value>800</value></transactionShares>
        <transactionPricePerShare><value>0</value></transactionPricePerShare>
        <transactionAcquiredDisposedCode><value>A</value></transactionAcquiredDisposedCode>
      </transactionAmounts>
    </nonDerivativeTransaction>
    <nonDerivativeHolding>
      <securityTitle><value>Action ordinaire</value></securityTitle>
      <postTransactionAmounts>
        <sharesOwnedFollowingTransaction><value>4200</value></sharesOwnedFollowingTransaction>
      </postTransactionAmounts>
      <ownershipNature>
        <directOrIndirectOwnership><value>I</value></directOrIndirectOwnership>
        <natureOfOwnership><value>Par un trust familial</value></natureOfOwnership>
      </ownershipNature>
    </nonDerivativeHolding>
  </nonDerivativeTable>
  <derivativeTable>
    <derivativeTransaction>
      <securityTitle><value>Option d'achat</value></securityTitle>
      <conversionOrExercisePrice><value>31.25</value></conversionOrExercisePrice>
      <transactionDate><value>2026-08-10</value></transactionDate>
      <transactionCoding><transactionCode>M</transactionCode></transactionCoding>
      <transactionTimeliness><value>L</value></transactionTimeliness>
      <transactionAmounts>
        <transactionShares><value>9999</value></transactionShares>
        <transactionAcquiredDisposedCode><value>D</value></transactionAcquiredDisposedCode>
      </transactionAmounts>
      <exerciseDate><value>2024-01-15</value></exerciseDate>
      <expirationDate><value>2030-01-15</value></expirationDate>
      <underlyingSecurity>
        <underlyingSecurityTitle><value>Action ordinaire</value></underlyingSecurityTitle>
        <underlyingSecurityShares><value>9999</value></underlyingSecurityShares>
      </underlyingSecurity>
      <ownershipNature>
        <directOrIndirectOwnership><value>D</value></directOrIndirectOwnership>
      </ownershipNature>
    </derivativeTransaction>
    <derivativeHolding>
      <securityTitle><value>Unité d'action restreinte</value></securityTitle>
      <postTransactionAmounts>
        <sharesOwnedFollowingTransaction><value>12000</value></sharesOwnedFollowingTransaction>
      </postTransactionAmounts>
    </derivativeHolding>
  </derivativeTable>
  <remarks>Rectification du nombre de titres.</remarks>
  <ownerSignature><signatureDate>2026-08-13</signatureDate></ownerSignature>
</ownershipDocument>
"""

FLUX = """<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>4 - Martin Claire (0001) (Reporting)</title>
    <link rel="alternate" type="text/html"
          href="https://www.sec.gov/Archives/edgar/data/1/000111/0001-26-1-index.htm"/>
  </entry>
  <entry>
    <title>4 - Exemple Corp (0002) (Issuer)</title>
    <link rel="alternate" type="text/html"
          href="https://www.sec.gov/Archives/edgar/data/2/000111/0001-26-1-index.htm"/>
  </entry>
  <entry>
    <title>424B2 - Banque Machin (0003) (Filer)</title>
    <link rel="alternate" type="text/html"
          href="https://www.sec.gov/Archives/edgar/data/3/000222/0002-26-2-index.htm"/>
  </entry>
  <entry>
    <title>4/A - Dupont Jean (0004) (Reporting)</title>
    <link rel="alternate" type="text/html"
          href="https://www.sec.gov/Archives/edgar/data/4/000333/0003-26-3-index.htm"/>
  </entry>
</feed>
"""


# ---=== Flux EDGAR ===---

def test_le_flux_ecarte_les_formulaires_qui_ne_sont_pas_des_4():
    """EDGAR traite `type=4` comme un préfixe : le flux ramène aussi les 424B2.

    Un prospectus n'a aucune transaction d'initié. Sans ce filtre, le lecteur
    part chercher un `ownershipDocument` qui n'existe pas.
    """
    trouves = sec_data.extraire_depots(FLUX)
    types = [t for t, _, _ in trouves]
    assert "424B2" not in types
    assert types == ["4", "4", "4/A"]


def test_un_depot_apparait_deux_fois_dans_le_flux():
    """Une entrée sous le CIK de l'émetteur, une sous celui du déclarant.

    Les deux pointent le même numéro d'accession. Sans dédoublonnage en aval,
    chaque transaction est comptée en double — ce qui doublerait tous les totaux.
    """
    urls = [u for _, _, u in sec_data.extraire_depots(FLUX)]
    accessions = [u.rsplit("/", 2)[-2] for u in urls]
    assert accessions.count("000111") == 2      # le même dépôt, deux entrées


def test_flux_vide_ne_plante_pas():
    vide = '<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"></feed>'
    assert sec_data.extraire_depots(vide) == []


# ---=== Lecture d'un formulaire 4 ===---

def test_parser_lit_les_transactions_sur_titres():
    lignes = sec_data.parser_formulaire4(FORMULAIRE, source="essai.xml")
    # Deux transactions sur actions, une détention, et les deux lignes dérivées.
    assert len(lignes) == 5
    assert lignes[0]["source"] == "essai.xml"

    achat = lignes[0]
    assert achat["ticker"] == "XMPL"             # normalisé en majuscules
    assert achat["societe"] == "Exemple Corp"
    assert achat["declarant"] == "Martin Claire"
    assert achat["role"] == "Directrice financière"
    assert achat["code"] == "P"
    assert achat["nature"] == "decision"
    assert achat["sens"] == 1                    # acquis
    assert achat["titres"] == 1500
    assert achat["montant"] == pytest.approx(1500 * 42.50)
    assert achat["detenu_apres"] == 18200


def test_les_quatre_tables_sont_lues():
    """Tout est extrait : transactions et détentions, actions et dérivés. Une
    donnée jamais extraite est perdue ; un filtre à la lecture se change."""
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    vues = {(l["categorie"], l["evenement"]) for l in lignes}
    assert vues == {("action", "transaction"), ("action", "detention"),
                    ("derive", "transaction"), ("derive", "detention")}


def test_les_derives_portent_ce_qui_leur_est_propre():
    """Prix d'exercice, dates et sous-jacent : sans eux, une option déclarée ne
    dit ni ce qu'elle vaut ni sur quoi elle porte."""
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    option = next(l for l in lignes
                  if l["categorie"] == "derive" and l["evenement"] == "transaction")
    assert option["prix_exercice"] == 31.25
    assert option["date_expiration"] == "2030-01-15"
    assert option["sous_jacent"] == "Action ordinaire"
    assert option["titres_sous_jacent"] == 9999.0
    # Le montant reste celui de l'option, jamais du sous-jacent : les mêler
    # produirait des totaux qui ne veulent rien dire.
    assert option["montant"] == 0


def test_les_actions_n_ont_pas_les_colonnes_des_derives():
    """Une action n'a ni prix d'exercice ni échéance : les remplir de zéros les
    ferait passer pour des options exerçables sur-le-champ."""
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    action = next(l for l in lignes if l["categorie"] == "action")
    assert "prix_exercice" not in action


def test_la_detention_indirecte_dit_par_qui():
    """« I » seul ne dit rien : natureOfOwnership distingue un trust d'un
    conjoint, et souvent deux lignes autrement identiques."""
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    detenu = next(l for l in lignes if l["evenement"] == "detention"
                  and l["categorie"] == "action")
    assert detenu["detention"] == "indirecte"
    assert detenu["nature_detention"] == "Par un trust familial"
    assert detenu["detenu_apres"] == 4200.0
    # Une détention ne déplace rien : le signe doit être neutre.
    assert detenu["sens"] == 0


def test_une_detention_herite_de_la_date_du_rapport():
    """Sans date propre, elle disparaîtrait de tout tri chronologique."""
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    detenu = next(l for l in lignes if l["evenement"] == "detention")
    assert detenu["date"] == "2026-08-11"


def test_le_depot_tardif_est_signale():
    """Deux jours ouvrés, c'est la règle. Le formulaire ne le dit que lorsqu'il
    est en retard : la case vide signifie « dans les délais »."""
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    option = next(l for l in lignes if l["categorie"] == "derive"
                  and l["evenement"] == "transaction")
    assert option["ponctualite"] == "tardive"
    action = next(l for l in lignes if l["code"] == "P")
    assert action["ponctualite"] == "dans les délais"


def test_l_identite_du_declarant_est_complete():
    """Le CIK identifie sans ambiguïté là où deux homonymes se confondraient."""
    ligne = sec_data.parser_formulaire4(FORMULAIRE)[0]
    assert ligne["cik_declarant"] == "0001111111"
    assert ligne["cik_emetteur"] == "0000320193"
    assert ligne["ville_declarant"] == "Cupertino"
    assert ligne["etat_declarant"] == "CA"
    assert ligne["type_formulaire"] == "4"


def test_les_qualites_cumulees_sont_gardees_a_part():
    """Un initié peut être administrateur ET détenteur de plus de 10 % : le rôle
    résumé en choisit un, les drapeaux les gardent tous."""
    ligne = sec_data.parser_formulaire4(FORMULAIRE)[0]
    assert ligne["est_dirigeant"] is True
    assert ligne["est_administrateur"] is False
    assert ligne["role"] == "Directrice financière"


def test_les_remarques_et_la_signature_sont_lues():
    ligne = sec_data.parser_formulaire4(FORMULAIRE)[0]
    assert ligne["remarques"] == "Rectification du nombre de titres."
    assert ligne["date_signature"] == "2026-08-13"


def test_la_nature_separe_la_decision_de_la_remuneration():
    """C'est le tri qui porte la valeur : la plupart des formulaires 4 ne sont
    pas des décisions d'investissement."""
    assert sec_data.decrire_code("P")[1] == "decision"
    assert sec_data.decrire_code("S")[1] == "decision"
    assert sec_data.decrire_code("A")[1] == "remuneration"
    assert sec_data.decrire_code("M")[1] == "mecanique"
    assert sec_data.decrire_code("F")[1] == "mecanique"
    assert sec_data.decrire_code("ZZ")[1] == "autre"


def test_le_plan_10b5_1_est_detecte_dans_les_notes():
    """Le drapeau n'est pas toujours renseigné ; la mention en note fait foi.

    Une vente programmée des mois à l'avance ne dit rien de ce que l'initié
    pense aujourd'hui.
    """
    avec_note = FORMULAIRE.replace(
        "</ownershipDocument>",
        "<footnotes><footnote>Vente effectuée dans le cadre d'un plan 10b5-1 "
        "adopté le 3 mars.</footnote></footnotes></ownershipDocument>")
    assert all(l["programme_10b5_1"] for l in sec_data.parser_formulaire4(avec_note))
    assert not any(l["programme_10b5_1"] for l in sec_data.parser_formulaire4(FORMULAIRE))


def test_role_deduit_du_titre_puis_des_drapeaux():
    sans_titre = FORMULAIRE.replace(
        "<officerTitle>Directrice financière</officerTitle>", "")
    assert sec_data.parser_formulaire4(sans_titre)[0]["role"] == "dirigeant"

    detenteur = (FORMULAIRE.replace("<isOfficer>1</isOfficer>", "<isOfficer>0</isOfficer>")
                 .replace("<isTenPercentOwner>0</isTenPercentOwner>",
                          "<isTenPercentOwner>1</isTenPercentOwner>"))
    assert sec_data.parser_formulaire4(detenteur)[0]["role"] == "détenteur > 10 %"


# ---=== Mise en trame et agrégats ===---

def test_en_trame_garde_les_detentions_par_defaut():
    """Une position détenue sans mouvement dit ce que l'initié possède, ce
    qu'aucune transaction ne raconte."""
    df = sec_data.en_trame(sec_data.parser_formulaire4(FORMULAIRE))
    assert len(df) == 5
    assert (df.evenement == "detention").sum() == 2
    assert pd.api.types.is_datetime64_any_dtype(df.date)


def test_en_trame_peut_ecarter_ce_qui_n_est_pas_un_flux():
    """Pour sommer des montants il faut des opérations : une détention porte zéro
    titre échangé et tirerait les moyennes vers le bas."""
    df = sec_data.en_trame(sec_data.parser_formulaire4(FORMULAIRE),
                           seulement_transactions=True)
    assert len(df) == 3
    assert (df.evenement == "transaction").all()


def test_les_colonnes_de_dates_sont_typees():
    """Les cinq dates d'un formulaire, pas seulement celle de la transaction."""
    df = sec_data.en_trame(sec_data.parser_formulaire4(FORMULAIRE))
    for colonne in ("date", "date_expiration", "date_signature", "periode"):
        assert pd.api.types.is_datetime64_any_dtype(df[colonne]), colonne


def test_en_trame_vide_reste_utilisable():
    df = sec_data.en_trame([])
    assert df.empty and "ticker" in df.columns


def test_resume_compte_les_declarants_pas_les_lignes():
    """Un achat isolé ne dit rien ; plusieurs dirigeants la même semaine, si.

    C'est pour ça que l'agrégat compte les personnes distinctes et pas le nombre
    d'opérations, qu'un seul initié peut gonfler en découpant son ordre.
    """
    base = sec_data.parser_formulaire4(FORMULAIRE)[0]
    lignes = [
        {**base, "declarant": "Martin Claire", "code": "P", "montant": 60_000},
        {**base, "declarant": "Martin Claire", "code": "P", "montant": 40_000},
        {**base, "declarant": "Dupont Jean", "code": "P", "montant": 25_000},
        {**base, "declarant": "Roux Alain", "code": "S", "montant": 30_000},
    ]
    resume = sec_data.resume_par_societe(sec_data.en_trame(lignes))
    ligne = resume.iloc[0]
    assert ligne.operations == 4
    assert ligne.declarants == 3
    assert ligne.acheteurs == 2                  # deux personnes ont acheté, pas trois
    assert ligne.achats == pytest.approx(125_000)
    assert ligne.ventes == pytest.approx(30_000)
    assert ligne.net == pytest.approx(95_000)


def test_user_agent_a_un_contact_par_defaut(monkeypatch):
    """La SEC renvoie 403 sans contact analysable. Refuser de partir faute de
    configuration ferait échouer un script qui a tout ce qu'il lui faut."""
    monkeypatch.delenv(sec_data.VARIABLE_UA, raising=False)
    assert "@" in sec_data.user_agent()

    # Un contact sans adresse ne vaut rien : on retombe sur celui du dépôt.
    monkeypatch.setenv(sec_data.VARIABLE_UA, "juste un nom sans adresse")
    assert sec_data.user_agent() == sec_data.CONTACT_DEFAUT

    # Et la variable l'emporte quand elle est utilisable.
    monkeypatch.setenv(sec_data.VARIABLE_UA, "Dylan B dylan@exemple.fr")
    assert sec_data.user_agent() == "Dylan B dylan@exemple.fr"
