"""Transactions d'initiés, depuis les formulaires 4 déposés à la SEC.

Aucune source privée : dirigeants, administrateurs et détenteurs de plus de 10 %
sont tenus de déclarer leurs opérations dans les deux jours ouvrés. Les dépôts
sont publics, structurés en XML, et servis par EDGAR sans clé ni compte.

    python sec_data.py              # les derniers dépôts, tous codes
    python sec_data.py --achats     # seulement les achats sur le marché ouvert

CE QUI PORTE UN SIGNAL, ET CE QUI N'EN PORTE PAS

La majorité des formulaires 4 ne sont pas des décisions d'investissement. Une
attribution d'actions est une rémunération ; un exercice d'options suivi d'une
retenue fiscale est mécanique ; une vente est souvent programmée des mois à
l'avance dans un plan 10b5-1. Ce qui reste discutable comme signal, c'est l'ACHAT
sur le marché ouvert — l'initié engage son propre argent, à un moment qu'il
choisit. D'où le classement de chaque code en « décision », « rémunération »,
« mécanique » ou « autre », et le filtre --achats.

LIMITES
  - deux jours ouvrés de délai réglementaire : ce n'est jamais du temps réel ;
  - un plan 10b5-1 signalé retire tout caractère décisionnel à une opération, et
    le drapeau n'est pas toujours renseigné ;
  - la SEC demande un User-Agent identifiant avec un contact, et limite le débit.
"""

import argparse
import xml.etree.ElementTree as ET
from datetime import datetime, timezone

import pandas as pd
import requests

ATOM = "https://www.sec.gov/cgi-bin/browse-edgar"
TICKERS = "https://www.sec.gov/files/company_tickers.json"

# La SEC exige un User-Agent contenant un contact analysable, et renvoie 403 sinon
# — vérifié : une chaîne sans adresse est refusée, une adresse suffit. On le lit
# dans l'environnement plutôt que de le coder en dur : un contact bidon fait
# bannir l'IP, et l'adresse personnelle n'a rien à faire dans un dépôt public.
VARIABLE_UA = "SEC_USER_AGENT"


def user_agent():
    """User-Agent déclaré à la SEC, ou une erreur qui dit quoi faire."""
    import os

    valeur = os.environ.get(VARIABLE_UA, "").strip()
    if "@" not in valeur:
        raise ValueError(
            f"La SEC exige un contact dans le User-Agent, sinon elle renvoie 403.\n"
            f"  export {VARIABLE_UA}=\"Ton Nom ton.adresse@exemple.fr\"\n"
            f"Mets une adresse que tu relèves : c'est par là qu'ils préviennent "
            f"avant de bloquer.")
    return valeur

# (libellé, nature). La nature est ce qui décide de la lecture : seules les
# opérations « décision » engagent un choix de l'initié.
CODES = {
    "P": ("achat sur le marché ouvert", "decision"),
    "S": ("vente sur le marché ouvert", "decision"),
    "A": ("attribution de titres", "remuneration"),
    "M": ("exercice d'options", "mecanique"),
    "F": ("titres retenus pour l'impôt", "mecanique"),
    "C": ("conversion", "mecanique"),
    "X": ("exercice de bons", "mecanique"),
    "E": ("expiration courte", "mecanique"),
    "H": ("expiration longue", "mecanique"),
    "G": ("donation", "autre"),
    "D": ("cession à l'émetteur", "autre"),
    "I": ("opération hors marché", "autre"),
    "J": ("autre opération", "autre"),
    "K": ("échange", "autre"),
    "L": ("petite acquisition", "autre"),
    "U": ("offre publique", "autre"),
    "V": ("opération déclarée volontairement", "autre"),
    "W": ("succession", "autre"),
    "Z": ("dépôt en trust", "autre"),
}


def decrire_code(code):
    return CODES.get(str(code).upper(), ("code inconnu", "autre"))


def session_sec(agent=None):
    s = requests.Session()
    s.headers.update({"User-Agent": agent or user_agent(),
                      "Accept-Encoding": "gzip, deflate"})
    return s


# ---=== Analyse d'un dépôt (fonctions pures, testables hors ligne) ===---

def _texte(noeud, chemin):
    trouve = noeud.find(chemin) if noeud is not None else None
    return trouve.text.strip() if trouve is not None and trouve.text else None


def _nombre(noeud, chemin):
    brut = _texte(noeud, chemin)
    if brut is None:
        return None
    try:
        return float(brut.replace(",", ""))
    except ValueError:
        return None


def _role(relation):
    """Rôle du déclarant, du plus significatif au moins."""
    if relation is None:
        return "non précisé"
    titre = _texte(relation, "officerTitle")
    drapeaux = {c: _texte(relation, c) in ("1", "true") for c in
                ("isOfficer", "isDirector", "isTenPercentOwner", "isOther")}
    if drapeaux["isOfficer"] and titre:
        return titre
    if drapeaux["isOfficer"]:
        return "dirigeant"
    if drapeaux["isTenPercentOwner"]:
        return "détenteur > 10 %"
    if drapeaux["isDirector"]:
        return "administrateur"
    return "non précisé"


def parser_formulaire4(xml_texte, source=None):
    """Un formulaire 4 -> une ligne par transaction sur titres non dérivés.

    Les tables dérivées (options, bons) sont écartées : elles mélangent des
    attributions et des exercices dont la valeur en dollars n'est pas comparable
    à un achat d'actions, et les additionner produirait des totaux faux.

    Fonction pure : elle prend le XML, pas une URL. C'est ce qui la rend testable
    sans toucher au réseau.
    """
    racine = ET.fromstring(xml_texte)
    emetteur = racine.find(".//issuer")
    proprietaire = racine.find(".//reportingOwner")

    commun = {
        "societe": _texte(emetteur, "issuerName"),
        "ticker": (_texte(emetteur, "issuerTradingSymbol") or "").upper() or None,
        "cik_emetteur": _texte(emetteur, "issuerCik"),
        "declarant": _texte(proprietaire, ".//rptOwnerName"),
        "role": _role(racine.find(".//reportingOwnerRelationship")),
        "periode": _texte(racine, "periodOfReport"),
        "source": source,
    }

    # Le drapeau 10b5-1 a changé de nom selon les versions du schéma, et reste
    # parfois seulement mentionné en note de bas de page.
    programme = any(
        (n.text or "").strip() in ("1", "true")
        for n in racine.iter() if "10b5" in n.tag.lower())
    if not programme:
        notes = " ".join((n.text or "") for n in racine.iter("footnote")).lower()
        programme = "10b5-1" in notes

    lignes = []
    for tr in racine.findall(".//nonDerivativeTransaction"):
        code = _texte(tr, ".//transactionCode")
        titres = _nombre(tr, ".//transactionShares/value")
        prix = _nombre(tr, ".//transactionPricePerShare/value")
        sens = _texte(tr, ".//transactionAcquiredDisposedCode/value")
        libelle, nature = decrire_code(code)
        lignes.append({
            **commun,
            "date": _texte(tr, ".//transactionDate/value"),
            "code": code,
            "operation": libelle,
            "nature": nature,
            # A = acquis, D = cédé. Le signe rend les totaux additionnables.
            "sens": 1 if sens == "A" else (-1 if sens == "D" else 0),
            "titres": titres,
            "prix": prix,
            "montant": (titres or 0) * (prix or 0),
            "detenu_apres": _nombre(tr, ".//sharesOwnedFollowingTransaction/value"),
            "titre": _texte(tr, ".//securityTitle/value"),
            "programme_10b5_1": programme,
        })
    return lignes


def en_trame(lignes):
    """Lignes -> trame typée, la plus récente d'abord."""
    colonnes = ["date", "ticker", "societe", "declarant", "role", "code", "operation",
                "nature", "sens", "titres", "prix", "montant", "detenu_apres",
                "programme_10b5_1", "periode", "cik_emetteur", "titre", "source"]
    df = pd.DataFrame(lignes, columns=colonnes)
    if df.empty:
        return df
    df["date"] = pd.to_datetime(df["date"], errors="coerce")
    for colonne in ("titres", "prix", "montant", "detenu_apres", "sens"):
        df[colonne] = pd.to_numeric(df[colonne], errors="coerce")
    # Une opération sans titres échangés n'est pas une transaction exploitable
    df = df[df.titres.fillna(0) > 0]
    return df.sort_values("date", ascending=False).reset_index(drop=True)


# ---=== Accès EDGAR ===---

ATOM_NS = {"a": "http://www.w3.org/2005/Atom"}


def extraire_depots(flux_atom, types_voulus=("4", "4/A")):
    """Flux Atom -> [(type, titre, url)] filtré sur le type EXACT.

    EDGAR interprète `type=4` comme un préfixe : le flux ramène aussi les 424B2,
    qui sont des prospectus sans la moindre transaction d'initié. Le type réel est
    en tête du titre de chaque entrée, sous la forme « 4 - Nom (CIK) (Reporting) ».

    Fonction pure : elle prend le flux, pas une URL.
    """
    racine = ET.fromstring(flux_atom)
    sortie = []
    for entree in racine.findall("a:entry", ATOM_NS):
        titre = (entree.findtext("a:title", default="", namespaces=ATOM_NS) or "").strip()
        lien = entree.find("a:link", ATOM_NS)
        url = lien.get("href") if lien is not None else None
        type_formulaire = titre.split(" - ", 1)[0].strip()
        if url and type_formulaire in types_voulus:
            sortie.append((type_formulaire, titre, url))
    return sortie


def derniers_depots(nombre=40, session=None, type_formulaire="4"):
    """URL des derniers formulaires 4 déposés, via le flux Atom d'EDGAR."""
    session = session or session_sec()
    reponse = session.get(ATOM, timeout=30, params={
        "action": "getcurrent", "type": type_formulaire, "output": "atom",
        "count": min(max(nombre * 3, 40), 100),   # marge : le flux mélange les types
        "company": "", "dateb": "", "owner": "include"})
    reponse.raise_for_status()

    # Un formulaire 4 apparaît DEUX fois dans le flux : une entrée sous le CIK de
    # l'émetteur, une sous celui du déclarant, pour le même numéro d'accession.
    # Sans dédoublonnage, chaque transaction est comptée en double.
    vus, sortie = set(), []
    for _, _, url in extraire_depots(reponse.text):
        accession = url.rsplit("/", 2)[-2] if "/" in url else url
        if accession in vus:
            continue
        vus.add(accession)
        sortie.append(url)
        if len(sortie) >= nombre:
            break
    return sortie


def _xml_du_depot(url_index, session):
    """Document XML structuré d'un dépôt, via l'index JSON du dossier.

    Le listing HTML du dossier ne rend plus les liens de fichiers de façon
    exploitable ; `index.json` donne la liste réelle, ce qui évite de deviner.
    """
    dossier = url_index.rsplit("/", 1)[0] + "/"
    index = session.get(dossier + "index.json", timeout=30)
    if index.status_code != 200:
        return None, dossier
    noms = [item.get("name", "") for item in index.json().get("directory", {}).get("item", [])]
    xmls = [n for n in noms if n.lower().endswith(".xml")]
    # Le document primaire porte les transactions ; les autres sont des annexes.
    ordre = [n for n in xmls if "primary_doc" in n] + [n for n in xmls if "primary_doc" not in n]
    for nom in ordre:
        doc = session.get(dossier + nom, timeout=30)
        if doc.status_code == 200 and "ownershipDocument" in doc.text:
            return doc.text, dossier + nom
    return None, dossier


def fetch_insiders(nombre=40, session=None, seulement_achats=False):
    """Derniers formulaires 4 -> trame de transactions.

    Un dépôt illisible est ignoré plutôt que de faire échouer l'ensemble : EDGAR
    sert aussi de vieux formats et des dépôts corrigés.
    """
    session = session or session_sec()
    lignes = []
    for url in derniers_depots(nombre, session):
        try:
            xml, adresse = _xml_du_depot(url, session)
            if xml:
                lignes.extend(parser_formulaire4(xml, source=adresse))
        except (requests.RequestException, ET.ParseError):
            continue

    df = en_trame(lignes)
    if seulement_achats and not df.empty:
        df = df[(df.code == "P") & (~df.programme_10b5_1)].reset_index(drop=True)
    return df


def resume_par_societe(df, minimum=1):
    """Agrège par société : c'est là que le signal se lit, pas ligne à ligne.

    Un achat isolé ne dit pas grand-chose ; plusieurs dirigeants qui achètent la
    même semaine, si. On compte donc les déclarants distincts, pas les lignes.
    """
    if df.empty:
        return df
    achats = df[df.code == "P"]
    ventes = df[df.code == "S"]
    par = df.groupby(["ticker", "societe"], dropna=False).agg(
        operations=("code", "size"),
        declarants=("declarant", "nunique"),
        derniere=("date", "max"),
    )
    par["achats"] = achats.groupby(["ticker", "societe"], dropna=False).montant.sum()
    par["ventes"] = ventes.groupby(["ticker", "societe"], dropna=False).montant.sum()
    par[["achats", "ventes"]] = par[["achats", "ventes"]].fillna(0.0)
    par["net"] = par.achats - par.ventes
    par["acheteurs"] = achats.groupby(["ticker", "societe"], dropna=False).declarant.nunique()
    par["acheteurs"] = par["acheteurs"].fillna(0).astype(int)
    par = par[par.operations >= minimum]
    return par.reset_index().sort_values("net", ascending=False)


def main():
    p = argparse.ArgumentParser(description="Transactions d'initiés (formulaires 4 SEC)")
    p.add_argument("--nombre", type=int, default=40, help="dépôts à lire (défaut : 40)")
    p.add_argument("--achats", action="store_true",
                   help="ne garder que les achats sur le marché ouvert hors plan 10b5-1")
    args = p.parse_args()

    df = fetch_insiders(args.nombre, seulement_achats=args.achats)
    if df.empty:
        print("aucune transaction exploitable dans les derniers dépôts")
        return
    maintenant = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    print(f"{len(df)} transactions, relevé du {maintenant}\n")
    colonnes = ["date", "ticker", "declarant", "role", "operation", "titres", "prix", "montant"]
    apercu = df[colonnes].head(25).copy()
    apercu["date"] = apercu.date.dt.strftime("%Y-%m-%d")
    print(apercu.to_string(index=False, max_colwidth=28))


if __name__ == "__main__":
    try:
        main()
    except (requests.RequestException, ValueError) as err:
        raise SystemExit(f"Erreur : {err}")
