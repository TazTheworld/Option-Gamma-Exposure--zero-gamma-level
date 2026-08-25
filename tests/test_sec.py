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
  <periodOfReport>2026-08-11</periodOfReport>
  <issuer>
    <issuerCik>0000320193</issuerCik>
    <issuerName>Exemple Corp</issuerName>
    <issuerTradingSymbol>xmpl</issuerTradingSymbol>
  </issuer>
  <reportingOwner>
    <reportingOwnerId><rptOwnerName>Martin Claire</rptOwnerName></reportingOwnerId>
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
  </nonDerivativeTable>
  <derivativeTable>
    <derivativeTransaction>
      <transactionCoding><transactionCode>M</transactionCode></transactionCoding>
      <transactionAmounts><transactionShares><value>9999</value></transactionShares></transactionAmounts>
    </derivativeTransaction>
  </derivativeTable>
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
    assert len(lignes) == 2                      # la table dérivée est écartée

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


def test_les_options_ne_sont_pas_melangees_aux_actions():
    """La table dérivée mêle attributions et exercices, dont la valeur en dollars
    n'est pas comparable à un achat d'actions. Les additionner fausserait tout."""
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    assert all(l["titres"] != 9999 for l in lignes)


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

def test_en_trame_ecarte_les_lignes_sans_titres():
    lignes = sec_data.parser_formulaire4(FORMULAIRE)
    lignes.append({**lignes[0], "titres": 0})
    df = sec_data.en_trame(lignes)
    assert len(df) == 2
    assert pd.api.types.is_datetime64_any_dtype(df.date)


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


def test_user_agent_sans_contact_refuse_avec_un_message_actionnable(monkeypatch):
    """La SEC renvoie 403 sans contact analysable — vérifié en conditions réelles."""
    monkeypatch.delenv(sec_data.VARIABLE_UA, raising=False)
    with pytest.raises(ValueError, match="SEC_USER_AGENT"):
        sec_data.user_agent()

    monkeypatch.setenv(sec_data.VARIABLE_UA, "juste un nom sans adresse")
    with pytest.raises(ValueError, match="contact"):
        sec_data.user_agent()

    monkeypatch.setenv(sec_data.VARIABLE_UA, "Dylan B dylan@exemple.fr")
    assert sec_data.user_agent() == "Dylan B dylan@exemple.fr"
