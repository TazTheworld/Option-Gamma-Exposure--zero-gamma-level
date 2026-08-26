"""Déclarations d'initiés, depuis les formulaires 4 déposés à la SEC.

Aucune source privée : dirigeants, administrateurs et détenteurs de plus de 10 %
sont tenus de déclarer leurs opérations dans les deux jours ouvrés. Les dépôts
sont publics, structurés en XML, et servis par EDGAR sans clé ni compte.

    python sec_data.py                      # aperçu des derniers dépôts
    python sec_data.py --csv initiés.csv    # les 45 colonnes
    python sec_data.py --achats             # les achats sur le marché ouvert
    python sec_data.py --transactions       # sans les positions détenues

TOUT EST EXTRAIT

Les quatre tables du formulaire sont lues — transactions et détentions, sur
actions et sur dérivés — avec les champs annexes : identité et adresse du
déclarant, détention directe ou indirecte et par quel intermédiaire, ponctualité
du dépôt, prix d'exercice et échéance des options, notes de bas de page,
remarques, signature.

Le tri se fait APRÈS, par les colonnes `nature`, `categorie` et `evenement`. Un
filtre à la lecture se change ; une donnée jamais extraite demande de tout
redemander à EDGAR.

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
  - les montants ne s'additionnent qu'entre actions : le « prix » d'un dérivé est
    celui de l'option, pas du titre sous-jacent ;
  - la SEC limite le débit et exige un contact joignable dans le User-Agent.
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


# Contact déclaré à la SEC. Elle l'exige — un User-Agent sans adresse joignable
# renvoie 403 — et s'en sert pour prévenir avant de bloquer un client trop
# gourmand. Ce doit donc être une adresse réellement relevée.
CONTACT_DEFAUT = "Taz broissartdylan0@gmail.com"


def user_agent():
    """User-Agent déclaré à la SEC.

    La variable d'environnement l'emporte, pour qu'un autre utilisateur du dépôt
    déclare son propre contact sans toucher au code. Sans elle, celui du dépôt
    sert : refuser de partir faute de configuration ferait échouer un script qui
    a tout ce qu'il lui faut.
    """
    import os

    valeur = os.environ.get(VARIABLE_UA, "").strip()
    return valeur if "@" in valeur else CONTACT_DEFAUT

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


def _drapeaux(relation):
    """Les quatre qualités déclarées, telles quelles.

    Un initié peut être plusieurs choses à la fois — administrateur ET détenteur
    de plus de 10 %, par exemple. Les réduire à un seul rôle perd de
    l'information : les drapeaux bruts vivent donc À CÔTÉ du rôle résumé, et ce
    sont eux qu'on filtre.
    """
    if relation is None:
        return {"dirigeant": False, "administrateur": False,
                "detenteur_10pct": False, "autre": False, "titre_officier": None}

    def vrai(champ):
        return _texte(relation, champ) in ("1", "true")

    return {
        "dirigeant": vrai("isOfficer"),
        "administrateur": vrai("isDirector"),
        "detenteur_10pct": vrai("isTenPercentOwner"),
        "autre": vrai("isOther"),
        "titre_officier": _texte(relation, "officerTitle") or _texte(relation, "otherText"),
    }


def _role(relation):
    """Rôle du déclarant, du plus significatif au moins.

    Un résumé lisible, pas une donnée : quand un initié cumule les qualités,
    celle-ci en choisit une seule. Pour trier, il faut les colonnes `est_*`.
    """
    d = _drapeaux(relation)
    if d["dirigeant"] and d["titre_officier"]:
        return d["titre_officier"]
    if d["dirigeant"]:
        return "dirigeant"
    if d["detenteur_10pct"]:
        return "détenteur > 10 %"
    if d["administrateur"]:
        return "administrateur"
    if d["autre"]:
        return d["titre_officier"] or "autre"
    return "non précisé"


def _valeur(noeud, chemin):
    """La valeur d'un champ, qu'il soit enveloppé dans `value` ou non.

    Le schéma enveloppe la plupart des champs, mais pas tous, et pas dans toutes
    ses versions. Chercher les deux évite de perdre un champ selon l'âge du dépôt.
    """
    return _texte(noeud, chemin + "/value") or _texte(noeud, chemin)


def _forme_de_detention(noeud):
    """Directe ou indirecte, et par quel intermédiaire.

    « D » se lit en son nom propre, « I » par un trust, un conjoint, une société.
    L'intermédiaire est en clair dans `natureOfOwnership`, et c'est souvent la
    seule chose qui distingue deux lignes autrement identiques.
    """
    forme = _valeur(noeud, ".//directOrIndirectOwnership")
    return {
        "detention": {"D": "directe", "I": "indirecte"}.get(forme, forme),
        "nature_detention": _valeur(noeud, ".//natureOfOwnership"),
    }


def _ponctualite(code):
    """La déclaration a-t-elle été faite dans les temps ?

    Deux jours ouvrés, c'est la règle. Un dépôt tardif est une information en
    soi — et le formulaire ne le dit que lorsqu'il l'est : la case vide signifie
    « dans les délais », pas « non renseigné ».
    """
    return {"E": "anticipée", "L": "tardive"}.get(code, "dans les délais")


def _lignes_d_une_table(racine, balise, categorie, evenement, commun):
    """Toutes les lignes d'une table du formulaire.

    Les quatre tables — transactions et détentions, sur actions et sur dérivés —
    ont la même ossature. Les traiter séparément dupliquerait vingt extractions
    identiques et laisserait les quatre diverger au premier champ ajouté.
    """
    lignes = []
    for noeud in racine.findall(".//" + balise):
        code = _valeur(noeud, ".//transactionCoding/transactionCode")
        libelle, nature = decrire_code(code) if code else ("détention", "detention")
        titres = _nombre(noeud, ".//transactionShares/value")
        prix = _nombre(noeud, ".//transactionPricePerShare/value")
        sens = _valeur(noeud, ".//transactionAcquiredDisposedCode")

        ligne = dict(commun)
        ligne.update({
            "categorie": categorie,
            "evenement": evenement,
            # Une détention n'a pas de date propre : elle vaut à la date du
            # rapport, et laisser la case vide la ferait disparaître de tout tri
            # chronologique.
            "date": _valeur(noeud, ".//transactionDate") or commun.get("periode"),
            "code": code,
            "operation": libelle,
            "nature": nature,
            # A = acquis, D = cédé. Le signe rend les totaux additionnables ; une
            # détention ne déplace rien, donc zéro.
            "sens": 1 if sens == "A" else (-1 if sens == "D" else 0),
            "titres": titres,
            "prix": prix,
            "montant": (titres or 0) * (prix or 0),
            "titre": _valeur(noeud, ".//securityTitle"),
            "detenu_apres": _nombre(noeud, ".//sharesOwnedFollowingTransaction/value"),
            # Certains dérivés déclarent une valeur en dollars plutôt qu'un nombre
            # de titres : l'ignorer ferait passer la position pour nulle.
            "valeur_detenue_apres": _nombre(noeud, ".//valueOwnedFollowingTransaction/value"),
            "date_execution_reputee": _valeur(noeud, ".//deemedExecutionDate"),
            "ponctualite": _ponctualite(_valeur(noeud, ".//transactionTimeliness")),
            "swap_actions": _valeur(noeud, ".//equitySwapInvolved") in ("1", "true"),
        })
        ligne.update(_forme_de_detention(noeud))

        if categorie == "derive":
            # Ce qui n'existe que sur un dérivé, et qui décide de ce qu'il vaut :
            # à quel prix il se convertit, quand il devient exerçable, et sur
            # combien de titres il porte.
            ligne.update({
                "prix_exercice": _nombre(noeud, ".//conversionOrExercisePrice/value"),
                "date_exercice": _valeur(noeud, ".//exerciseDate"),
                "date_expiration": _valeur(noeud, ".//expirationDate"),
                "sous_jacent": _valeur(noeud, ".//underlyingSecurityTitle"),
                "titres_sous_jacent": _nombre(noeud, ".//underlyingSecurityShares/value"),
                "valeur_sous_jacent": _nombre(noeud, ".//underlyingSecurityValue/value"),
            })
        lignes.append(ligne)
    return lignes


# Les quatre tables d'un formulaire 4, et ce qu'elles décrivent.
TABLES = [
    ("nonDerivativeTransaction", "action", "transaction"),
    ("nonDerivativeHolding", "action", "detention"),
    ("derivativeTransaction", "derive", "transaction"),
    ("derivativeHolding", "derive", "detention"),
]


def parser_formulaire4(xml_texte, source=None):
    """Un formulaire 4 -> une ligne par événement déclaré.

    **Tout est extrait**, y compris ce qui ne relève d'aucune décision
    d'investissement : les attributions, les exercices mécaniques, les dérivés et
    les positions simplement détenues. Le tri se fait après, par les colonnes
    `nature`, `categorie` et `evenement` — un filtre à la lecture se change, une
    donnée jamais extraite est perdue.

    Fonction pure : elle prend le XML, pas une URL. C'est ce qui la rend testable
    sans toucher au réseau.
    """
    racine = ET.fromstring(xml_texte)
    emetteur = racine.find(".//issuer")
    proprietaire = racine.find(".//reportingOwner")
    relation = racine.find(".//reportingOwnerRelationship")
    adresse = racine.find(".//reportingOwnerAddress")
    drapeaux = _drapeaux(relation)

    notes = [(n.text or "").strip() for n in racine.iter("footnote")]
    notes = [n for n in notes if n]

    # Le drapeau 10b5-1 a changé de nom selon les versions du schéma, et reste
    # parfois seulement mentionné en note de bas de page.
    programme = any(
        (n.text or "").strip() in ("1", "true")
        for n in racine.iter() if "10b5" in n.tag.lower())
    if not programme:
        programme = "10b5-1" in " ".join(notes).lower()

    commun = {
        "societe": _texte(emetteur, "issuerName"),
        "ticker": (_texte(emetteur, "issuerTradingSymbol") or "").upper() or None,
        "cik_emetteur": _texte(emetteur, "issuerCik"),
        "declarant": _texte(proprietaire, ".//rptOwnerName"),
        "cik_declarant": _texte(proprietaire, ".//rptOwnerCik"),
        "role": _role(relation),
        "est_dirigeant": drapeaux["dirigeant"],
        "est_administrateur": drapeaux["administrateur"],
        "est_detenteur_10pct": drapeaux["detenteur_10pct"],
        "est_autre": drapeaux["autre"],
        "ville_declarant": _texte(adresse, "rptOwnerCity"),
        "etat_declarant": _texte(adresse, "rptOwnerState"),
        "periode": _texte(racine, "periodOfReport"),
        "type_formulaire": _texte(racine, "documentType"),
        # Un amendement corrige un dépôt antérieur : le confondre avec une
        # opération neuve compterait deux fois la même transaction.
        "amendement": _texte(racine, "dateOfOriginalSubmission") is not None,
        "hors_section_16": _texte(racine, "notSubjectToSection16") in ("1", "true"),
        "date_signature": _texte(racine, ".//signatureDate"),
        "remarques": _texte(racine, "remarks"),
        "notes": " | ".join(notes) or None,
        "programme_10b5_1": programme,
        "source": source,
    }

    lignes = []
    for balise, categorie, evenement in TABLES:
        lignes.extend(_lignes_d_une_table(racine, balise, categorie, evenement, commun))
    return lignes


# L'ordre d'affichage, et le contrat de la trame. Les colonnes propres aux
# dérivés viennent en dernier : elles sont vides sur la plupart des lignes.
COLONNES = [
    "date", "ticker", "societe", "declarant", "role", "categorie", "evenement",
    "code", "operation", "nature", "sens", "titres", "prix", "montant",
    "detenu_apres", "valeur_detenue_apres", "detention", "nature_detention",
    "programme_10b5_1", "ponctualite", "swap_actions", "titre",
    "est_dirigeant", "est_administrateur", "est_detenteur_10pct", "est_autre",
    "ville_declarant", "etat_declarant", "cik_emetteur", "cik_declarant",
    "periode", "type_formulaire", "amendement", "hors_section_16",
    "date_execution_reputee", "date_signature", "remarques", "notes",
    "prix_exercice", "date_exercice", "date_expiration",
    "sous_jacent", "titres_sous_jacent", "valeur_sous_jacent",
    "source",
]

DATES = ("date", "date_execution_reputee", "date_exercice", "date_expiration",
         "date_signature", "periode")

NOMBRES = ("titres", "prix", "montant", "detenu_apres", "sens",
           "valeur_detenue_apres", "prix_exercice", "titres_sous_jacent",
           "valeur_sous_jacent")


def en_trame(lignes, seulement_transactions=False):
    """Lignes -> trame typée, la plus récente d'abord.

    Les détentions sont **gardées par défaut** : une position détenue sans
    mouvement dit ce que l'initié possède, ce qu'aucune transaction ne raconte.
    Elles portent zéro titre échangé, donc `seulement_transactions` les écarte
    quand on veut sommer des flux — sans quoi les moyennes seraient tirées vers
    le bas par des lignes qui ne sont pas des opérations.
    """
    df = pd.DataFrame(lignes, columns=COLONNES)
    if df.empty:
        return df
    for colonne in DATES:
        df[colonne] = pd.to_datetime(df[colonne], errors="coerce")
    for colonne in NOMBRES:
        df[colonne] = pd.to_numeric(df[colonne], errors="coerce")
    if seulement_transactions:
        df = df[(df.evenement == "transaction") & (df.titres.fillna(0) > 0)]
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


def fetch_insiders(nombre=40, session=None, seulement_achats=False,
                   seulement_transactions=False):
    """Derniers formulaires 4 -> trame de tout ce qui y est déclaré.

    Tout est rendu par défaut : transactions et détentions, actions et dérivés,
    décisions et attributions. Les deux filtres restreignent, ils n'ajoutent rien
    — ce qui n'a pas été extrait ne se récupère pas sans redemander à EDGAR.

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

    df = en_trame(lignes, seulement_transactions=seulement_transactions)
    if seulement_achats and not df.empty:
        # L'achat sur le marché ouvert, hors plan programmé : le seul cas où
        # l'initié engage son argent à un moment qu'il choisit.
        df = df[(df.code == "P") & (~df.programme_10b5_1)].reset_index(drop=True)
    return df


def resume_par_societe(df, minimum=1):
    """Agrège par société : c'est là que le signal se lit, pas ligne à ligne.

    Un achat isolé ne dit pas grand-chose ; plusieurs dirigeants qui achètent la
    même semaine, si. On compte donc les déclarants distincts, pas les lignes.
    """
    if df.empty:
        return df
    # Les montants ne s'additionnent qu'entre actions : le « prix » d'un dérivé
    # est celui de l'option, pas du titre, et les mêler produirait des totaux
    # qui ne veulent rien dire. Les détentions, elles, ne sont pas des flux.
    flux = df[(df.categorie == "action") & (df.evenement == "transaction")]
    if flux.empty:
        return flux
    achats = flux[flux.code == "P"]
    ventes = flux[flux.code == "S"]
    par = flux.groupby(["ticker", "societe"], dropna=False).agg(
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
    p = argparse.ArgumentParser(
        description="Déclarations d'initiés (formulaires 4 SEC) — tout ce qui y figure")
    p.add_argument("--nombre", type=int, default=40, help="dépôts à lire (défaut : 40)")
    p.add_argument("--achats", action="store_true",
                   help="ne garder que les achats sur le marché ouvert hors plan 10b5-1")
    p.add_argument("--transactions", action="store_true",
                   help="écarter les positions simplement détenues")
    p.add_argument("--csv", metavar="FICHIER",
                   help="écrire toutes les colonnes dans un CSV plutôt qu'un aperçu")
    args = p.parse_args()

    df = fetch_insiders(args.nombre, seulement_achats=args.achats,
                        seulement_transactions=args.transactions)
    if df.empty:
        print("aucune déclaration exploitable dans les derniers dépôts")
        return

    if args.csv:
        # Les 45 colonnes, pas l'aperçu : c'est le point de tout extraire.
        df.to_csv(args.csv, index=False)
        print(f"{len(df)} lignes, {len(df.columns)} colonnes -> {args.csv}")
        return

    maintenant = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    print(f"{len(df)} déclarations, relevé du {maintenant}")
    # Ce que l'aperçu cache : le détail est dans le CSV, et le dire évite de
    # croire que la trame se limite à ces huit colonnes.
    par_categorie = df.groupby(["categorie", "evenement"]).size()
    print("  " + " | ".join(f"{k[0]} {k[1]} : {v}" for k, v in par_categorie.items()))
    print(f"  {len(df.columns)} colonnes au total — --csv pour les avoir toutes")
    print()

    colonnes = ["date", "ticker", "declarant", "role", "operation", "titres",
                "prix", "detention"]
    apercu = df[colonnes].head(25).copy()
    apercu["date"] = apercu.date.dt.strftime("%Y-%m-%d")
    print(apercu.to_string(index=False, max_colwidth=26))


if __name__ == "__main__":
    try:
        main()
    except (requests.RequestException, ValueError) as err:
        raise SystemExit(f"Erreur : {err}")
