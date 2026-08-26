"""Transactions boursières des sénateurs américains.

Même loi que pour la Chambre — le STOCK Act de 2012 — mais un tout autre système
de dépôt. Le Sénat sert ses déclarations en HTML plutôt qu'en PDF, ce qui rend
l'extraction plus simple et plus sûre : un tableau reste un tableau, là où un PDF
coupe ses lignes n'importe où.

    python senat_data.py                     # aperçu des derniers rapports
    python senat_data.py --depuis 01/01/2026
    python senat_data.py --csv senat.csv     # toutes les colonnes

CE QUI DIFFÈRE DE LA CHAMBRE

| | Chambre | Sénat |
|---|---|---|
| Index | un ZIP annuel | une recherche paginée |
| Rapport | PDF, texte extrait | HTML, tableau |
| Symbole | entre parenthèses dans l'actif | colonne dédiée |

Les colonnes rendues sont **les mêmes**, à dessein : les deux chambres se
concatènent alors sans retouche, et la colonne `chambre` suffit à les distinguer.

L'ACCÈS DEMANDE UNE ACCEPTATION, ET ELLE EST EXPLICITE

Le Sénat fait accepter une clause d'usage avant toute recherche. Ce module la
transmet — c'est ce que fait un navigateur — mais ne la masque pas : elle est
envoyée par une fonction qui porte son nom, et le message le rappelle au premier
appel. Accepter en votre nom sans le dire serait le genre de silence que ce dépôt
s'interdit ailleurs.

Le service filtre par ailleurs les adresses de centres de données : une machine
domestique passe, un serveur non. Ce n'est pas un refus d'automatisation mais un
filtrage de réputation, et le module ne cherche pas à le contourner — il échoue
en le disant.

LIMITES
  - quarante-cinq jours de délai réglementaire, comme pour la Chambre ;
  - les montants sont des fourchettes, jamais des valeurs exactes ;
  - les dépôts papier scannés n'ont pas de tableau : ils sont signalés, pas
    devinés.
"""

import argparse
import re
from datetime import datetime, timezone
from html.parser import HTMLParser

import pandas as pd
import requests

RACINE = "https://efdsearch.senate.gov"
ACCUEIL = RACINE + "/search/home/"
RECHERCHE = RACINE + "/search/report/data/"

# Le type de rapport dans le formulaire de recherche. 11 = rapport de
# transactions périodique, le seul qui porte des opérations.
TYPE_TRANSACTIONS = 11

CONTACT_DEFAUT = "Taz broissartdylan0@gmail.com"
VARIABLE_UA = "SENATE_USER_AGENT"


def user_agent():
    import os

    valeur = os.environ.get(VARIABLE_UA, "").strip()
    return valeur if valeur else CONTACT_DEFAUT


class AccesRefuse(RuntimeError):
    """Le service a refusé la connexion.

    Distinguée d'une erreur réseau ordinaire parce que la cause est presque
    toujours la même et que le remède n'est pas d'insister : le Sénat filtre les
    adresses de centres de données. Depuis une machine domestique, ça passe.
    """


def _verifier(reponse):
    if reponse.status_code == 403:
        raise AccesRefuse(
            "efdsearch.senate.gov a refusé la connexion (403). Le service filtre "
            "les adresses de centres de données ; une machine domestique passe. "
            "Vérifie en ouvrant " + ACCUEIL + " dans un navigateur.")
    reponse.raise_for_status()
    return reponse


JETON = re.compile(r'name="csrfmiddlewaretoken"\s+value="([^"]+)"')


def accepter_les_conditions(session=None, silencieux=False):
    """Ouvre une session en acceptant la clause d'usage du Sénat.

    Le service la fait valider avant toute recherche. La transmettre est ce que
    fait un navigateur ; la transmettre **sans le dire** serait un silence que ce
    dépôt s'interdit — d'où le message, et le nom de cette fonction.
    """
    session = session or requests.Session()
    session.headers.update({"User-Agent": user_agent()})

    accueil = _verifier(session.get(ACCUEIL, timeout=45))
    jeton = JETON.search(accueil.text)
    if not jeton:
        raise RuntimeError(
            "Jeton d'acceptation introuvable sur la page d'accueil du Sénat. "
            "Le formulaire a probablement changé.")

    if not silencieux:
        print("Clause d'usage du Sénat acceptée pour cette session.")
    _verifier(session.post(
        ACCUEIL, timeout=45,
        data={"prohibition_agreement": "1", "csrfmiddlewaretoken": jeton.group(1)},
        headers={"Referer": ACCUEIL}))
    session.headers["Referer"] = RACINE + "/search/"
    return session, jeton.group(1)


# ---=== La liste des rapports ===---

LIEN = re.compile(r'href="([^"]+)"[^>]*>(.*?)</a>', re.DOTALL)
BALISES = re.compile(r"<[^>]+>")


def parser_recherche(charge):
    """La réponse de recherche -> une ligne par rapport déposé.

    Le service rend un tableau au format DataTables : `data` est une liste de
    lignes, chacune une liste de cellules — prénom, nom, fonction, lien, date.
    Le lien porte à la fois le libellé du rapport et son adresse, et c'est de lui
    qu'on tire le type de dépôt.

    Fonction pure : elle prend la charge déjà décodée, pas une URL.
    """
    lignes = []
    for cellules in (charge or {}).get("data", []):
        if len(cellules) < 5:
            continue
        prenom, nom, fonction, lien, date = cellules[:5]
        trouve = LIEN.search(lien or "")
        adresse = trouve.group(1) if trouve else None
        libelle = BALISES.sub("", trouve.group(2)).strip() if trouve else None
        lignes.append({
            "nom": " ".join(BALISES.sub("", x or "").strip() for x in (prenom, nom)).strip(),
            "fonction": BALISES.sub("", fonction or "").strip() or None,
            "rapport": libelle,
            # Un dépôt papier se reconnaît à son adresse : il n'a pas de tableau,
            # seulement un scan, et le lire ne rendrait rien.
            "papier": bool(adresse and "/paper/" in adresse),
            "url": RACINE + adresse if adresse and adresse.startswith("/") else adresse,
            "date_depot": BALISES.sub("", date or "").strip() or None,
        })
    return lignes


# ---=== Le rapport lui-même ===---

class _Tableau(HTMLParser):
    """Extrait les lignes du premier tableau rencontré.

    `html.parser` de la bibliothèque standard plutôt qu'une dépendance : un
    tableau à neuf colonnes ne justifie pas d'ajouter un analyseur complet, et le
    dépôt tient ses dépendances courtes.
    """

    def __init__(self):
        super().__init__()
        self.lignes, self._ligne, self._cellule, self._dans = [], [], [], False

    def handle_starttag(self, balise, attrs):
        if balise == "tr":
            self._ligne = []
        elif balise in ("td", "th"):
            self._dans, self._cellule = True, []

    def handle_endtag(self, balise):
        if balise in ("td", "th"):
            self._ligne.append(" ".join("".join(self._cellule).split()))
            self._dans = False
        elif balise == "tr" and self._ligne:
            self.lignes.append(self._ligne)

    def handle_data(self, texte):
        if self._dans:
            self._cellule.append(texte)


# Les colonnes du tableau, telles que le Sénat les intitule. On les repère par
# leur titre et non par leur position : une colonne insérée en tête décalerait
# tout sans qu'aucun test ne le voie.
ENTETES = {
    "transaction date": "date",
    "owner": "detenteur",
    "ticker": "symbole",
    "asset name": "actif",
    "asset type": "nature_titre",
    "type": "code_operation",
    "amount": "fourchette",
    "comment": "commentaire",
}

DETENTEURS = {
    "self": "l'élu",
    "spouse": "conjoint",
    "child": "enfant à charge",
    "joint": "compte joint",
    "": "l'élu",
}

OPERATIONS = {
    "purchase": ("achat", 1),
    "sale (full)": ("vente", -1),
    "sale (partial)": ("vente partielle", -1),
    "sale": ("vente", -1),
    "exchange": ("échange", 0),
}

FOURCHETTE = re.compile(r"\$([\d,]+)\s*-\s*\$([\d,]+)")
PLAFOND_OUVERT = re.compile(r"Over\s+\$([\d,]+)", re.IGNORECASE)


def decrire_detenteur(brut):
    return DETENTEURS.get((brut or "").strip().lower(), brut or "l'élu")


def decrire_operation(brut):
    return OPERATIONS.get((brut or "").strip().lower(), (brut or None, 0))


def _entier(brut):
    try:
        return float(brut.replace(",", ""))
    except (AttributeError, ValueError):
        return None


def _bornes(montant):
    """Les deux bornes d'une fourchette déclarée.

    Une fourchette n'est pas un montant. « Over $50,000,000 » n'a pas de
    plafond : en inventer un fausserait toute somme.
    """
    trouve = FOURCHETTE.search(montant or "")
    if trouve:
        return _entier(trouve.group(1)), _entier(trouve.group(2))
    ouvert = PLAFOND_OUVERT.search(montant or "")
    if ouvert:
        return _entier(ouvert.group(1)), None
    return None, None


NOM = re.compile(r"<h[12][^>]*>(.*?)</h[12]>", re.DOTALL | re.IGNORECASE)


def parser_rapport(html, url=None, nom=None):
    """Le HTML d'un rapport -> une ligne par transaction.

    Les colonnes sont repérées par leur **titre**, pas par leur position : une
    colonne insérée en tête décalerait toute la lecture sans qu'aucun test ne le
    voie, et les montants se retrouveraient dans la colonne du type.

    Fonction pure : elle prend le HTML, pas une URL.
    """
    analyseur = _Tableau()
    analyseur.feed(html or "")
    if not analyseur.lignes:
        return []

    entete = [c.strip().lower() for c in analyseur.lignes[0]]
    place = {}
    for i, titre in enumerate(entete):
        for cle, champ in ENTETES.items():
            if titre.startswith(cle):
                place[champ] = i
    if "date" not in place or "code_operation" not in place:
        # Sans date ni sens, ce n'est pas un tableau de transactions : rendre des
        # lignes vides ferait passer un rapport illisible pour un rapport sans
        # opération.
        return []

    if nom is None:
        titre = NOM.search(html or "")
        nom = BALISES.sub("", titre.group(1)).strip() if titre else None

    operations = []
    for cellules in analyseur.lignes[1:]:
        def valeur(champ):
            i = place.get(champ)
            return cellules[i].strip() if i is not None and i < len(cellules) else None

        montant = valeur("fourchette")
        bas, haut = _bornes(montant)
        libelle, sens = decrire_operation(valeur("code_operation"))
        symbole = (valeur("symbole") or "").strip()
        operations.append({
            "chambre": "senat",
            "nom": nom,
            "date": valeur("date"),
            # Le Sénat ne publie pas de date de notification distincte : la
            # laisser vide dit la vérité, la recopier inventerait un délai nul.
            "date_notification": None,
            # « -- » remplit la case quand le titre n'est pas coté.
            "symbole": symbole if symbole and symbole not in ("--", "-") else None,
            "actif": valeur("actif"),
            "nature_titre": valeur("nature_titre"),
            "code_operation": valeur("code_operation"),
            "operation": libelle,
            "sens": sens,
            "detenteur": decrire_detenteur(valeur("detenteur")),
            "montant_min": bas,
            "montant_max": haut,
            "fourchette": montant,
            "description": valeur("commentaire") or None,
            "source": url,
        })
    return operations


COLONNES = [
    "date", "date_notification", "delai_jours", "nom", "chambre", "etat", "district",
    "symbole", "actif", "nature_titre", "operation", "code_operation", "sens",
    "detenteur", "montant_min", "montant_max", "montant_milieu", "fourchette",
    "compte", "statut", "description", "statut_elu", "doc_id", "source",
]


def en_trame(operations):
    """Opérations -> trame typée, la plus récente d'abord.

    Mêmes colonnes que pour la Chambre : les deux chambres se concatènent alors
    sans retouche, et `chambre` suffit à les distinguer.
    """
    df = pd.DataFrame(operations, columns=COLONNES)
    if df.empty:
        return df
    for colonne in ("date", "date_notification"):
        df[colonne] = pd.to_datetime(df[colonne], format="%m/%d/%Y", errors="coerce")
    for colonne in ("montant_min", "montant_max", "sens"):
        df[colonne] = pd.to_numeric(df[colonne], errors="coerce")
    df["delai_jours"] = (df.date_notification - df.date).dt.days
    df["montant_milieu"] = (df.montant_min + df.montant_max) / 2
    return df.sort_values("date", ascending=False).reset_index(drop=True)


# ---=== Accès au service ===---

def chercher_rapports(session, jeton, depuis="01/01/2026", limite=50):
    """Les rapports de transactions déposés depuis une date."""
    reponse = _verifier(session.post(
        RECHERCHE, timeout=45,
        data={
            "start": "0", "length": str(limite),
            "report_types": f"[{TYPE_TRANSACTIONS}]", "filer_types": "[]",
            "submitted_start_date": f"{depuis} 00:00:00", "submitted_end_date": "",
            "candidate_state": "", "senator_state": "", "office_id": "",
            "first_name": "", "last_name": "",
            "csrfmiddlewaretoken": jeton,
        }))
    return parser_recherche(reponse.json())


def fetch_senat(depuis="01/01/2026", nombre=25, pause=0.3):
    """Les derniers rapports de transactions du Sénat.

    Un rapport papier est compté et annoncé plutôt que deviné : un scan n'a pas
    de tableau, et rendre zéro transaction le ferait passer pour un rapport vide.
    """
    import time

    session, jeton = accepter_les_conditions()
    rapports = chercher_rapports(session, jeton, depuis, limite=max(nombre * 2, 50))

    operations, papier, illisibles = [], 0, 0
    for rapport in rapports:
        if len(operations) and len({o["source"] for o in operations}) >= nombre:
            break
        if rapport["papier"] or not rapport["url"]:
            papier += 1
            continue
        try:
            page = _verifier(session.get(rapport["url"], timeout=45))
        except requests.RequestException:
            illisibles += 1
            continue
        lignes = parser_rapport(page.text, url=rapport["url"], nom=rapport["nom"])
        if not lignes:
            illisibles += 1
        operations.extend(lignes)
        time.sleep(pause)

    df = en_trame(operations)
    df.attrs.update(papier=papier, illisibles=illisibles, rapports=len(rapports))
    return df


def main():
    p = argparse.ArgumentParser(
        description="Transactions boursières des sénateurs (STOCK Act)")
    p.add_argument("--depuis", default="01/01/2026", metavar="MM/JJ/AAAA",
                   help="date de dépôt la plus ancienne (défaut : 01/01/2026)")
    p.add_argument("--nombre", type=int, default=25, help="rapports à lire (défaut : 25)")
    p.add_argument("--csv", metavar="FICHIER", help="écrire toutes les colonnes")
    args = p.parse_args()

    df = fetch_senat(args.depuis, args.nombre)
    if df.empty:
        print("aucune transaction exploitable dans les derniers rapports")
        return

    if args.csv:
        df.to_csv(args.csv, index=False)
        print(f"{len(df)} lignes, {len(df.columns)} colonnes -> {args.csv}")
        return

    maintenant = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    print(f"{len(df)} transactions, relevé du {maintenant}")
    if df.attrs["papier"]:
        print(f"  {df.attrs['papier']} dépôt(s) papier, sans tableau exploitable")
    if df.attrs["illisibles"]:
        print(f"  {df.attrs['illisibles']} rapport(s) illisibles")
    print()

    colonnes = ["date", "nom", "symbole", "operation", "fourchette", "detenteur"]
    apercu = df[colonnes].head(25).copy()
    apercu["date"] = apercu.date.dt.strftime("%Y-%m-%d")
    print(apercu.to_string(index=False, max_colwidth=26))


if __name__ == "__main__":
    try:
        main()
    except (requests.RequestException, AccesRefuse, RuntimeError) as err:
        raise SystemExit(f"Erreur : {err}")
