"""Déclarations du STOCK Act : l'index du greffe, et les rapports de transactions.

Les jeux d'essai reproduisent ce que le greffe sert réellement, artefacts
compris : les libellés espacés caractère par caractère — « F\\x00\\x00 S\\x00: New »
pour « Filing Status: New » — les actifs coupés sur deux lignes, et les montants
tranchés en plein milieu. Ce sont eux qui cassent un parseur naïf, et les lisser
dans la fixture reviendrait à tester un format qui n'existe pas.

Aucun test ne touche au réseau.
"""

import pandas as pd
import pytest

import congres_data as C

INDEX = """<?xml version="1.0"?>
<FinancialDisclosure>
  <Member>
    <Prefix>Hon.</Prefix><Last>DelBene</Last><First>Suzan</First><Suffix></Suffix>
    <FilingType>P</FilingType><StateDst>WA01</StateDst><Year>2026</Year>
    <FilingDate>3/31/2026</FilingDate><DocID>20033835</DocID>
  </Member>
  <Member>
    <Prefix>Hon.</Prefix><Last>Roy</Last><First>Chip</First><Suffix></Suffix>
    <FilingType>A</FilingType><StateDst>TX21</StateDst><Year>2026</Year>
    <FilingDate>5/15/2026</FilingDate><DocID>20030001</DocID>
  </Member>
  <Member>
    <Prefix>Hon.</Prefix><Last>Kelly</Last><First>Mike</First><Suffix>Jr.</Suffix>
    <FilingType>X</FilingType><StateDst>PA16</StateDst><Year>2026</Year>
    <FilingDate>6/01/2026</FilingDate><DocID>20030002</DocID>
  </Member>
</FinancialDisclosure>
"""

# Reproduit le texte extrait d'un vrai PDF : espacement encodé par des octets
# nuls, actif sur deux lignes, montant coupé, et une section finale qui ne parle
# plus de transactions.
RAPPORT = (
    "P\x00\x00 T\x00\x00 R\x00\n"
    "Clerk of the House of Representatives - B81 Cannon Building\n"
    "F\x00\x00 I\x00\x00\n"
    "Name: Hon. Suzan K. DelBene\n"
    "Status: Member\n"
    "State/District: WA01\n"
    "T\x00\x00\x00\n"
    "ID Owner Asset Transaction\n"
    "Type\n"
    "Date Notification\n"
    "Date\n"
    "Amount Cap.\n"
    "Gains >\n"
    "$200?\n"
    "Amazon.com, Inc. - Common Stock\n"
    "(AMZN) [ST]\n"
    "P 03/16/2026 03/20/2026 $1,001 - $15,000\n"
    "F\x00\x00 S\x00\x00: New\n"
    "S\x00\x00\x00 O\x00: Putnam Investments\n"
    "D\x00\x00\x00\x00: 25 shares bought @ $209.40/share\n"
    "JT Arizona Indl Dev Auth Rev BDS\n"
    "Lincoln 5.00% Due Nov 1, 2029 [GS]\n"
    "S (partial) 01/13/2026 01/13/2026 $15,001 -\n"
    "$50,000\n"
    "F\x00\x00 S\x00\x00: New\n"
    "SP Tesla, Inc. - Common Stock (TSLA)\n"
    "[ST]\n"
    "S 02/02/2026 02/10/2026 Over $50,000,000\n"
    "F\x00\x00 S\x00\x00: New\n"
    "* For the complete list of asset type abbreviations, please visit\n"
    "I\x00\x00 V\x00 D\x00\n"
    "Fidelity Brokerage Services, LLC\n"
)


# ---=== L'index annuel ===---

def test_l_index_separe_l_etat_du_district():
    """« WA01 » : les séparer permet de regrouper par État sans redécouper à
    chaque usage."""
    lignes = C.parser_index(INDEX)
    assert lignes[0]["etat"] == "WA"
    assert lignes[0]["district"] == "01"


def test_l_index_traduit_les_types_de_depot():
    """Seul « P » porte des transactions. Les confondre remplirait la trame de
    lignes sans opération."""
    lignes = C.parser_index(INDEX)
    assert [l["depot"] for l in lignes] == [
        "rapport de transactions", "déclaration annuelle", "extension de délai"]
    assert sum(1 for l in lignes if l["type_depot"] == "P") == 1


def test_l_index_recompose_le_nom_avec_son_suffixe():
    lignes = C.parser_index(INDEX)
    assert lignes[0]["nom"] == "Suzan DelBene"
    assert lignes[2]["nom"] == "Mike Kelly Jr."


def test_l_index_ne_porte_aucune_transaction():
    """C'est le point de la découpe : l'index dit qui a déposé, le PDF dit quoi."""
    lignes = C.parser_index(INDEX)
    assert all("symbole" not in l and "montant_min" not in l for l in lignes)
    assert all(l["doc_id"] for l in lignes)


# ---=== Le rapport de transactions ===---

def test_l_espacement_encode_est_retire():
    """« F\\x00\\x00 S\\x00\\x00: New » est « Filing Status: New ». Sans nettoyage, il
    faudrait des motifs illisibles pour cacher un artefact de mise en page."""
    assert C.nettoyer("F\x00\x00 S\x00: New") == "F S: New"


def test_les_trois_operations_sont_lues():
    ops = C.parser_ptr(RAPPORT, doc_id="20033835")
    assert len(ops) == 3
    assert [o["operation"] for o in ops] == ["achat", "vente partielle", "vente"]
    assert [o["sens"] for o in ops] == [1, -1, -1]


def test_l_identite_du_deposant_n_est_pas_un_actif():
    """Sans repère de début de tableau, le premier actif absorbe l'adresse du
    greffe et le nom de l'élu."""
    ops = C.parser_ptr(RAPPORT)
    assert ops[0]["actif"] == "Amazon.com, Inc. - Common Stock (AMZN) [ST]"
    assert "Cannon Building" not in (ops[0]["actif"] or "")
    assert ops[0]["nom"] == "Hon. Suzan K. DelBene"
    assert ops[0]["etat"] == "WA" and ops[0]["district"] == "01"


def test_un_actif_coupe_sur_deux_lignes_est_recolle():
    """Le symbole est sur la seconde ligne : ne lire que la première le perdrait."""
    ops = C.parser_ptr(RAPPORT)
    assert ops[0]["symbole"] == "AMZN"
    assert ops[0]["nature_titre"] == "ST"
    assert ops[2]["symbole"] == "TSLA"


def test_un_montant_coupe_est_recolle_et_ne_deborde_pas():
    """« $15,001 - » puis « $50,000 ». Sans consommer la seconde ligne, son reste
    retombe dans l'actif suivant — avec le détenteur qui l'ouvre."""
    ops = C.parser_ptr(RAPPORT)
    assert ops[1]["montant_min"] == 15_001
    assert ops[1]["montant_max"] == 50_000
    # Et l'opération d'après n'a pas hérité du débordement
    assert "$50,000" not in (ops[2]["actif"] or "")
    assert ops[2]["detenteur"] == "conjoint"


def test_le_detenteur_ouvre_la_ligne_d_actif():
    """L'absence de code signifie l'élu lui-même, ce qui n'est pas la même chose
    qu'une donnée manquante."""
    ops = C.parser_ptr(RAPPORT)
    assert ops[0]["detenteur"] == "l'élu"
    assert ops[1]["detenteur"] == "compte joint"
    assert ops[2]["detenteur"] == "conjoint"
    # Et le code ne reste pas collé à l'actif
    assert not (ops[1]["actif"] or "").startswith("JT")


def test_un_plafond_ouvert_n_est_pas_invente():
    """« Over $50,000,000 » n'a pas de plafond : en fabriquer un fausserait toute
    somme, et laisser le champ vide dit la vérité."""
    ops = C.parser_ptr(RAPPORT)
    assert ops[2]["montant_min"] == 50_000_000
    assert ops[2]["montant_max"] is None


def test_les_precisions_reviennent_a_leur_operation():
    """Le compte dit qui exécute réellement, et c'est souvent ce qui distingue un
    ordre choisi d'un arbitrage subi."""
    ops = C.parser_ptr(RAPPORT)
    assert ops[0]["compte"] == "Putnam Investments"
    assert ops[0]["description"] == "25 shares bought @ $209.40/share"
    assert ops[0]["statut"] == "New"
    # La deuxième n'a pas de compte : ne pas hériter de celui de la première.
    assert ops[1].get("compte") is None


def test_ce_qui_suit_le_tableau_est_ignore():
    """Les sections finales parlent d'introductions en bourse et de dettes, pas
    de transactions."""
    ops = C.parser_ptr(RAPPORT)
    assert all("Fidelity Brokerage" not in (o["actif"] or "") for o in ops)


def test_une_obligation_sans_symbole_reste_lisible():
    """Les obligations municipales n'ont pas de ticker : les écarter perdrait la
    moitié des lignes de certains rapports."""
    ops = C.parser_ptr(RAPPORT)
    assert ops[1]["symbole"] is None
    assert ops[1]["nature_titre"] == "GS"
    assert "Arizona" in ops[1]["actif"]


def test_un_rapport_vide_ne_plante_pas():
    assert C.parser_ptr("") == []
    assert C.parser_ptr("aucune structure reconnaissable") == []


# ---=== La trame ===---

def test_la_trame_calcule_le_delai_de_notification():
    """L'écart est une information en soi : un compte géré par un tiers notifie
    tard, un ordre passé en propre notifie le jour même."""
    df = C.en_trame(C.parser_ptr(RAPPORT))
    par_symbole = df.set_index("symbole").delai_jours
    assert par_symbole["AMZN"] == 4
    assert par_symbole["TSLA"] == 8


def test_le_milieu_n_existe_que_si_les_deux_bornes_existent():
    """Un milieu est une commodité, pas une mesure — et sur un plafond ouvert il
    n'en est même pas une."""
    df = C.en_trame(C.parser_ptr(RAPPORT))
    ferme = df[df.symbole == "AMZN"].iloc[0]
    assert ferme.montant_milieu == pytest.approx((1_001 + 15_000) / 2)
    ouvert = df[df.symbole == "TSLA"].iloc[0]
    assert pd.isna(ouvert.montant_milieu)


def test_la_trame_est_typee_et_triee():
    df = C.en_trame(C.parser_ptr(RAPPORT))
    assert pd.api.types.is_datetime64_any_dtype(df.date)
    assert pd.api.types.is_datetime64_any_dtype(df.date_notification)
    assert df.date.is_monotonic_decreasing


def test_une_trame_vide_reste_utilisable():
    df = C.en_trame([])
    assert df.empty and "symbole" in df.columns and "chambre" in df.columns


# ---=== Les codes ===---

def test_une_vente_partielle_n_est_pas_une_vente():
    """Les confondre ferait croire qu'une position a été soldée."""
    assert C.decrire_operation("S") == ("vente", -1)
    assert C.decrire_operation("S (partial)") == ("vente partielle", -1)
    # Un échange ne fait ni acquérir ni céder.
    assert C.decrire_operation("E")[1] == 0


def test_un_code_inconnu_est_rendu_tel_quel():
    """L'inventer ferait passer une lacune pour une donnée."""
    assert C.decrire_operation("Z") == ("Z", 0)
    assert C.decrire_depot("Z") == "inconnu"


def test_le_contact_a_un_defaut(monkeypatch):
    monkeypatch.delenv(C.VARIABLE_UA, raising=False)
    assert "@" in C.user_agent()
    monkeypatch.setenv(C.VARIABLE_UA, "Autre autre@exemple.fr")
    assert C.user_agent() == "Autre autre@exemple.fr"
