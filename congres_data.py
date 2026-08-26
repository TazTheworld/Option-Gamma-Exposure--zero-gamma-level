"""Transactions boursières des membres du Congrès américain.

Le STOCK Act de 2012 oblige les élus à déclarer leurs opérations sur titres dans
les quarante-cinq jours. Les dépôts sont publics et servis par le greffe de la
Chambre des représentants, sans clé ni compte.

    python congres_data.py                      # aperçu des derniers dépôts
    python congres_data.py --annee 2025         # une autre année
    python congres_data.py --csv congres.csv    # toutes les colonnes

CE QUE LA LOI IMPOSE, ET CE QU'ELLE N'IMPOSE PAS

Les montants sont déclarés en **fourchettes**, jamais en valeurs exactes :
« $1,001 - $15,000 » est ce que le formulaire contient, et aucun traitement ne
retrouvera le chiffre réel. Les bornes sont donc extraites séparément, et c'est
au lecteur de décider s'il raisonne sur le plancher, le plafond ou le milieu.

La description, elle, contient parfois le détail exact — nombre de titres et prix
unitaire — parce que l'élu a choisi de le donner. Elle est conservée telle quelle.

DEUX DATES, ET L'ÉCART ENTRE ELLES

`date` est celle de l'opération, `date_notification` celle où l'élu dit en avoir
été informé. Le délai de déclaration court à partir de la seconde, ce qui fait
que l'écart entre les deux est une information en soi : un compte géré par un
tiers notifie tard, un ordre passé en propre notifie le jour même.

LE SÉNAT N'EST PAS ICI

`efdsearch.senate.gov` refuse l'accès automatisé — tout le domaine `senate.gov`
répond 403, y compris ses pages institutionnelles. Le dépôt respecte ce refus,
comme il respecte celui de Stooq. Le module est bâti pour accueillir une seconde
chambre — d'où la colonne `chambre` — mais ne la fabrique pas.

LIMITES
  - quarante-cinq jours de délai réglementaire : ce n'est jamais du temps réel ;
  - une fourchette n'est pas un montant, et sommer des planchers sous-estime
    autant que sommer des plafonds surestime ;
  - un dépôt papier scanné n'a pas de texte extractible ; il est signalé, pas
    deviné.
"""

import argparse
import io
import re
import xml.etree.ElementTree as ET
import zipfile
from datetime import datetime, timezone

import pandas as pd
import requests

INDEX = "https://disclosures-clerk.house.gov/public_disc/financial-pdfs/{annee}FD.zip"
PTR = "https://disclosures-clerk.house.gov/public_disc/ptr-pdfs/{annee}/{doc}.pdf"

# Le greffe ne l'exige pas comme la SEC, mais s'identifier reste la moindre des
# choses quand on interroge un service public en boucle.
CONTACT_DEFAUT = "Taz broissartdylan0@gmail.com"
VARIABLE_UA = "HOUSE_USER_AGENT"


def user_agent():
    """En-tête déclaré au greffe de la Chambre."""
    import os

    valeur = os.environ.get(VARIABLE_UA, "").strip()
    return valeur if valeur else CONTACT_DEFAUT


def session_chambre(agent=None):
    session = requests.Session()
    session.headers.update({"User-Agent": agent or user_agent()})
    return session


# ---=== Ce que les codes veulent dire ===---

# Les types de dépôt de l'index annuel. Seul « P » porte des transactions ; les
# autres sont des déclarations de patrimoine, des extensions de délai ou des
# retraits, et les confondre remplirait la trame de lignes sans opération.
TYPES_DEPOT = {
    "P": "rapport de transactions",
    "A": "déclaration annuelle",
    "C": "déclaration de candidat",
    "D": "déclaration de départ",
    "T": "déclaration de fiducie",
    "W": "retrait",
    "X": "extension de délai",
    "H": "dépôt papier",
    "O": "autre",
}

# Qui détient le titre. L'absence de code signifie l'élu lui-même — ce qui n'est
# pas la même chose qu'une donnée manquante.
DETENTEURS = {
    "SP": "conjoint",
    "DC": "enfant à charge",
    "JT": "compte joint",
    "": "l'élu",
}

# Le sens de l'opération. « S (partial) » est une vente partielle : la distinguer
# d'une vente totale évite de croire qu'une position a été soldée.
OPERATIONS = {
    "P": ("achat", 1),
    "S": ("vente", -1),
    "S (partial)": ("vente partielle", -1),
    "E": ("échange", 0),
}


def decrire_depot(code):
    return TYPES_DEPOT.get((code or "").strip().upper(), "inconnu")


def decrire_detenteur(code):
    return DETENTEURS.get((code or "").strip().upper(), code)


def decrire_operation(code):
    return OPERATIONS.get((code or "").strip(), (code, 0))


# ---=== L'index annuel ===---

def parser_index(xml_texte):
    """L'index d'une année -> une ligne par déclaration déposée.

    Il ne contient AUCUNE transaction : seulement qui a déposé quoi et quand. Les
    opérations sont dans un PDF par déclaration, désigné par son `DocID`.

    Fonction pure : elle prend le XML, pas une URL.
    """
    racine = ET.fromstring(xml_texte)
    lignes = []
    for membre in racine:
        def champ(nom):
            valeur = membre.findtext(nom)
            return valeur.strip() if valeur else None

        type_depot = champ("FilingType")
        etat_district = champ("StateDst") or ""
        lignes.append({
            "nom": " ".join(filter(None, [champ("First"), champ("Last"), champ("Suffix")])),
            "prefixe": champ("Prefix"),
            "type_depot": type_depot,
            "depot": decrire_depot(type_depot),
            # « WA01 » : deux lettres d'État, deux chiffres de district. Les
            # séparer permet de regrouper par État sans découper à chaque usage.
            "etat": etat_district[:2] or None,
            "district": etat_district[2:] or None,
            "annee": champ("Year"),
            "date_depot": champ("FilingDate"),
            "doc_id": champ("DocID"),
        })
    return lignes


# ---=== Le rapport de transactions ===---

# Le PDF encode l'espacement des titres par des octets nuls : « Filing Status »
# y devient « F\x00\x00\x00\x00\x00 S\x00\x00\x00\x00\x00 ». Les retirer avant
# toute recherche évite d'écrire des motifs illisibles pour cacher un artefact.
def nettoyer(texte):
    """Le texte d'un PDF, débarrassé de son espacement encodé."""
    return re.sub(r"[\x00​]", "", texte or "")


# Une ligne de transaction : le sens, deux dates, puis la fourchette. C'est le
# seul motif fiable du document — les actifs, eux, sont coupés n'importe où.
LIGNE_OPERATION = re.compile(
    r"^(?P<sens>P|S \(partial\)|S|E)\s+"
    r"(?P<date>\d{2}/\d{2}/\d{4})\s+"
    r"(?P<notification>\d{2}/\d{2}/\d{4})\s+"
    r"(?P<montant>.*)$"
)

# « (AMZN) [ST] » : le symbole, puis le code de nature du titre. Le symbole
# manque sur les obligations municipales et les fonds non cotés.
SYMBOLE = re.compile(r"\(([A-Z][A-Z0-9.\-]{0,6})\)\s*(?:\[(\w{2})\])?")
NATURE_SEULE = re.compile(r"\[(\w{2})\]")

# Le détenteur ouvre la ligne d'actif quand il n'est pas l'élu lui-même.
DETENTEUR = re.compile(r"^(SP|DC|JT)\s+")

# « $1,001 - $15,000 », parfois coupé sur deux lignes par la mise en page.
FOURCHETTE = re.compile(r"\$([\d,]+)\s*-\s*\$([\d,]+)")
PLAFOND_OUVERT = re.compile(r"Over\s+\$([\d,]+)", re.IGNORECASE)

LIBELLES = {
    "Filing Status": "statut",
    "Subholding Of": "compte",
    "Description": "description",
}


def _entier(brut):
    try:
        return float(brut.replace(",", ""))
    except (AttributeError, ValueError):
        return None


def _bornes(montant):
    """Les deux bornes d'une fourchette déclarée.

    Une fourchette n'est pas un montant : sommer des planchers sous-estime autant
    que sommer des plafonds surestime. Les deux sont donc rendues, et le milieu
    avec — mais aucune n'est présentée comme « le » montant.
    """
    trouve = FOURCHETTE.search(montant or "")
    if trouve:
        bas, haut = _entier(trouve.group(1)), _entier(trouve.group(2))
        return bas, haut
    ouvert = PLAFOND_OUVERT.search(montant or "")
    if ouvert:
        # « Over $50,000,000 » n'a pas de plafond : en inventer un fausserait
        # toute somme, et laisser le champ vide dit la vérité.
        return _entier(ouvert.group(1)), None
    return None, None


def _entete(lignes):
    """Nom, statut et circonscription, en tête du rapport."""
    tete = {}
    for ligne in lignes[:15]:
        for cle, champ in (("Name:", "nom"), ("Status:", "statut_elu"),
                           ("State/District:", "etat_district")):
            if ligne.startswith(cle):
                tete[champ] = ligne.split(":", 1)[1].strip()
    return tete


# Ce qui clôt l'en-tête du document : la dernière colonne du tableau. Tant qu'on
# ne l'a pas vue, les lignes appartiennent à l'identité du déposant, pas aux
# actifs — sans ce repère, le premier actif absorbe l'adresse du greffe.
FIN_ENTETE = "$200?"

# Ce qui referme le tableau. Les sections suivantes parlent d'introductions en
# bourse et de dettes, pas de transactions.
FIN_TABLEAU = re.compile(r"^\*\s*For the complete list|^I\s*V\s*D|^I\s*P\s*O")


# Ce qui clôt l'en-tête du document : la dernière colonne du tableau. Tant qu'on
# ne l'a pas vue, les lignes appartiennent à l'identité du déposant, pas aux
# actifs — sans ce repère, le premier actif absorbe l'adresse du greffe.
FIN_ENTETE = "$200?"

# Ce qui referme le tableau. Les sections suivantes parlent d'introductions en
# bourse et de dettes, pas de transactions.
FIN_TABLEAU = re.compile(r"^\*\s*For the complete list|^I\s*V\s*D|^I\s*P\s*O")


def _est_un_libelle(ligne):
    """La ligne porte-t-elle « Filing Status », « Subholding Of » ou « Description » ?

    Les libellés sont espacés caractère par caractère dans le PDF : on ne compare
    donc que leurs initiales, dans l'ordre.
    """
    if ":" not in ligne:
        return False
    gauche = ligne.split(":", 1)[0]
    initiales = [mot[0] for mot in gauche.split() if mot]
    return any(initiales == [mot[0] for mot in prefixe.split()]
               for prefixe in LIBELLES)


def parser_ptr(texte, doc_id=None, chambre="representants"):
    """Le texte d'un rapport de transactions -> une ligne par opération.

    Le PDF n'a pas de structure exploitable : un actif s'étale sur deux lignes, un
    montant se coupe en plein milieu, et les libellés sont espacés caractère par
    caractère. Le seul repère fiable est la **ligne d'opération** — un sens, deux
    dates, une fourchette — et tout se lit par rapport à elle : l'actif est ce qui
    précède, les précisions sont ce qui suit.

    Fonction pure : elle prend le texte, pas un PDF ni une URL.
    """
    lignes = [l.strip() for l in nettoyer(texte).split("\n")]
    tete = _entete(lignes)
    etat_district = tete.get("etat_district", "") or ""

    commun = {
        "chambre": chambre,
        "nom": tete.get("nom"),
        "statut_elu": tete.get("statut_elu"),
        "etat": etat_district[:2] or None,
        "district": etat_district[2:] or None,
        "doc_id": doc_id,
    }

    operations = []
    tampon = []            # les lignes d'actif depuis la dernière opération
    dans_tableau = False
    consommee = -1         # ligne déjà absorbée par le montant précédent

    for i, ligne in enumerate(lignes):
        if not dans_tableau:
            # L'identité du déposant n'est pas un actif : tant que l'en-tête du
            # tableau n'est pas passé, rien ne s'accumule.
            if FIN_ENTETE in ligne:
                dans_tableau = True
            continue
        if FIN_TABLEAU.match(ligne):
            break
        if i == consommee:
            continue

        trouve = LIGNE_OPERATION.match(ligne)
        if not trouve:
            if _est_un_libelle(ligne):
                # Un libellé referme l'actif : ce qui suit ne lui appartient plus.
                tampon = []
                continue
            tampon.append(ligne)
            continue

        # Le montant peut déborder sur la ligne suivante : « $15,001 - » puis
        # « $50,000 ». La recoller ET la marquer consommée, sinon son reste
        # retombe dans l'actif de l'opération d'après — avec son détenteur.
        montant = trouve.group("montant").strip()
        if montant.endswith("-") and i + 1 < len(lignes):
            montant = montant + " " + lignes[i + 1]
            consommee = i + 1

        actif = " ".join(l for l in tampon if l).strip()
        tampon = []

        detenteur = ""
        marque = DETENTEUR.match(actif)
        if marque:
            detenteur = marque.group(1)
            actif = actif[marque.end():]

        symbole = SYMBOLE.search(actif)
        nature = symbole.group(2) if symbole and symbole.group(2) else None
        if nature is None:
            seule = NATURE_SEULE.search(actif)
            nature = seule.group(1) if seule else None

        libelle, sens = decrire_operation(trouve.group("sens"))
        bas, haut = _bornes(montant)

        operations.append({
            **commun,
            "date": trouve.group("date"),
            "date_notification": trouve.group("notification"),
            "symbole": symbole.group(1) if symbole else None,
            "actif": actif or None,
            "nature_titre": nature,
            "code_operation": trouve.group("sens"),
            "operation": libelle,
            # +1 acquis, -1 cédé, 0 pour un échange qui ne fait ni l'un ni
            # l'autre. Le signe rend les positions cumulables.
            "sens": sens,
            "detenteur": decrire_detenteur(detenteur),
            "montant_min": bas,
            "montant_max": haut,
            "fourchette": montant or None,
        })

    # Les précisions suivent leur opération : on les rattache après coup, parce
    # qu'elles n'apparaissent qu'une fois l'opération lue.
    _rattacher_precisions(lignes, operations)
    return operations


def _rattacher_precisions(lignes, operations):
    """Statut du dépôt, compte détenteur et description, rendus à leur opération.

    Le compte — « Subholding Of: Fidelity » — dit qui exécute réellement, et
    c'est souvent ce qui distingue un ordre choisi d'un arbitrage subi.
    """
    rang = -1
    for ligne in lignes:
        if LIGNE_OPERATION.match(ligne):
            rang += 1
            continue
        if rang < 0 or rang >= len(operations):
            continue
        for prefixe, champ in LIBELLES.items():
            initiale = prefixe[0]
            if ligne.startswith(initiale) and ":" in ligne:
                gauche, droite = ligne.split(":", 1)
                # Le libellé est espacé : on ne compare que ses initiales, dans
                # l'ordre. « F S » vaut « Filing Status », « S O » « Subholding Of ».
                if [m[0] for m in gauche.split()] == [m[0] for m in prefixe.split()]:
                    operations[rang][champ] = droite.strip() or None


COLONNES = [
    "date", "date_notification", "delai_jours", "nom", "chambre", "etat", "district",
    "symbole", "actif", "nature_titre", "operation", "code_operation", "sens",
    "detenteur", "montant_min", "montant_max", "montant_milieu", "fourchette",
    "compte", "statut", "description", "statut_elu", "doc_id", "source",
]


def en_trame(operations):
    """Opérations -> trame typée, la plus récente d'abord."""
    df = pd.DataFrame(operations, columns=COLONNES)
    if df.empty:
        return df
    for colonne in ("date", "date_notification"):
        df[colonne] = pd.to_datetime(df[colonne], format="%m/%d/%Y", errors="coerce")
    for colonne in ("montant_min", "montant_max", "sens"):
        df[colonne] = pd.to_numeric(df[colonne], errors="coerce")

    # Le délai entre l'opération et la notification : un compte géré par un tiers
    # notifie tard, un ordre passé en propre notifie le jour même.
    df["delai_jours"] = (df.date_notification - df.date).dt.days
    # Un milieu de fourchette est une commodité, pas une mesure : il n'existe que
    # si les deux bornes existent, et jamais sur un plafond ouvert.
    df["montant_milieu"] = (df.montant_min + df.montant_max) / 2
    return df.sort_values("date", ascending=False).reset_index(drop=True)


# ---=== Accès au greffe ===---

def telecharger_index(annee, session=None):
    """L'index d'une année, décompressé."""
    session = session or session_chambre()
    reponse = session.get(INDEX.format(annee=annee), timeout=60)
    reponse.raise_for_status()
    archive = zipfile.ZipFile(io.BytesIO(reponse.content))
    nom = next(n for n in archive.namelist() if n.lower().endswith(".xml"))
    # utf-8-sig : le greffe préfixe son XML d'une marque d'ordre des octets, que
    # le parseur refuserait.
    return archive.read(nom).decode("utf-8-sig")


def texte_du_ptr(annee, doc_id, session=None):
    """Le texte d'un rapport, ou None s'il n'en a pas.

    Un dépôt papier scanné n'a aucun texte extractible. Le dire vaut mieux que
    rendre une chaîne vide, qui se lirait comme un rapport sans transaction.
    """
    from pypdf import PdfReader

    session = session or session_chambre()
    reponse = session.get(PTR.format(annee=annee, doc=doc_id), timeout=60)
    if reponse.status_code == 404:
        return None
    reponse.raise_for_status()
    lecteur = PdfReader(io.BytesIO(reponse.content))
    texte = "\n".join((page.extract_text() or "") for page in lecteur.pages)
    return texte if texte.strip() else None


def fetch_congres(annee=None, nombre=40, session=None, pause=0.15):
    """Les derniers rapports de transactions d'une année.

    Un rapport illisible est ignoré plutôt que de faire échouer l'ensemble : le
    greffe sert aussi des dépôts papier scannés.

    `pause` espace les requêtes. Le greffe ne publie pas de limite de débit, mais
    en marteler un service public sans raison n'est pas une façon de faire.
    """
    import time

    annee = annee or datetime.now(timezone.utc).year
    session = session or session_chambre()
    index = parser_index(telecharger_index(annee, session))

    rapports = [d for d in index if d["type_depot"] == "P"]
    rapports.sort(key=lambda d: (d["date_depot"] or ""), reverse=True)

    operations, illisibles = [], 0
    for depot in rapports[:nombre]:
        try:
            texte = texte_du_ptr(annee, depot["doc_id"], session)
        except requests.RequestException:
            illisibles += 1
            continue
        if not texte:
            illisibles += 1
            continue
        lignes = parser_ptr(texte, doc_id=depot["doc_id"])
        for ligne in lignes:
            ligne["source"] = PTR.format(annee=annee, doc=depot["doc_id"])
        operations.extend(lignes)
        time.sleep(pause)

    df = en_trame(operations)
    df.attrs["illisibles"] = illisibles
    df.attrs["rapports"] = min(nombre, len(rapports))
    return df


def main():
    p = argparse.ArgumentParser(
        description="Transactions boursières des membres du Congrès (STOCK Act)")
    p.add_argument("--annee", type=int, help="année de dépôt (défaut : l'année en cours)")
    p.add_argument("--nombre", type=int, default=25,
                   help="rapports à lire (défaut : 25)")
    p.add_argument("--csv", metavar="FICHIER",
                   help="écrire toutes les colonnes dans un CSV plutôt qu'un aperçu")
    args = p.parse_args()

    df = fetch_congres(args.annee, args.nombre)
    if df.empty:
        print("aucune transaction exploitable dans les derniers rapports")
        return

    if args.csv:
        df.to_csv(args.csv, index=False)
        print(f"{len(df)} lignes, {len(df.columns)} colonnes -> {args.csv}")
        return

    maintenant = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    print(f"{len(df)} transactions sur {df.attrs['rapports']} rapports, "
          f"relevé du {maintenant}")
    if df.attrs["illisibles"]:
        # Annoncé, jamais tu en silence : un dépôt scanné n'a pas de texte, et le
        # taire ferait passer une lacune pour une absence d'opération.
        print(f"  {df.attrs['illisibles']} rapport(s) sans texte extractible (dépôts papier)")
    print(f"  {len(df.columns)} colonnes au total — --csv pour les avoir toutes")
    print()

    colonnes = ["date", "nom", "etat", "symbole", "operation", "fourchette",
                "detenteur", "delai_jours"]
    apercu = df[colonnes].head(25).copy()
    apercu["date"] = apercu.date.dt.strftime("%Y-%m-%d")
    print(apercu.to_string(index=False, max_colwidth=24))


if __name__ == "__main__":
    try:
        main()
    except (requests.RequestException, ValueError, zipfile.BadZipFile) as err:
        raise SystemExit(f"Erreur : {err}")
