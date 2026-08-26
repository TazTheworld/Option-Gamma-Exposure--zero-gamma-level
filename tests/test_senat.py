"""Rapports de transactions du Sénat : la recherche, et le tableau HTML.

Ces tests couvrent **le parsing**, pas l'accès. Le service n'accepte que les
clients qui se présentent comme un navigateur — voir ENTETES_HTTP dans le module — et
ce filtrage peut changer sans prévenir. C'est la même règle que pour TWS : ce qui
décide se teste hors ligne, ce qui parle au réseau se vérifie à la main.

Les fixtures reproduisent le format servi. Si le Sénat change son gabarit, ce
sont ces tests qui doivent échouer en premier, pas la collecte silencieusement.
"""

import pandas as pd
import pytest

import senat_data as S

RECHERCHE = {
    "data": [
        ["Jane", "<a href='/search/view/'>Doe</a>", "Senator",
         "<a href=\"/search/view/ptr/abc-123/\">Periodic Transaction Report for 03/2026</a>",
         "03/15/2026"],
        ["John", "Roe", "Senator",
         "<a href=\"/search/view/paper/def-456/\">Periodic Transaction Report</a>",
         "03/10/2026"],
    ]
}

RAPPORT = """
<html><body>
  <h1>Periodic Transaction Report for Jane Doe</h1>
  <table>
    <tr><th>#</th><th>Transaction Date</th><th>Owner</th><th>Ticker</th>
        <th>Asset Name</th><th>Asset Type</th><th>Type</th><th>Amount</th>
        <th>Comment</th></tr>
    <tr><td>1</td><td>03/02/2026</td><td>Spouse</td><td>NVDA</td>
        <td>NVIDIA Corporation</td><td>Stock</td><td>Purchase</td>
        <td>$15,001 - $50,000</td><td>--</td></tr>
    <tr><td>2</td><td>03/04/2026</td><td>Self</td><td>--</td>
        <td>Municipal Bond Fund</td><td>Other Securities</td><td>Sale (Partial)</td>
        <td>$1,001 - $15,000</td><td>Partial redemption</td></tr>
    <tr><td>3</td><td>03/05/2026</td><td>Joint</td><td>TSLA</td>
        <td>Tesla, Inc.</td><td>Stock</td><td>Sale (Full)</td>
        <td>Over $50,000,000</td><td>--</td></tr>
  </table>
</body></html>
"""

# Le même tableau, avec une colonne insérée en tête : la lecture par position
# décalerait tout sans qu'aucune erreur ne se voie.
RAPPORT_DECALE = RAPPORT.replace(
    "<tr><th>#</th><th>Transaction Date</th>",
    "<tr><th>Row</th><th>#</th><th>Transaction Date</th>").replace(
    "<tr><td>1</td><td>03/02/2026</td>", "<tr><td>a</td><td>1</td><td>03/02/2026</td>").replace(
    "<tr><td>2</td><td>03/04/2026</td>", "<tr><td>b</td><td>2</td><td>03/04/2026</td>").replace(
    "<tr><td>3</td><td>03/05/2026</td>", "<tr><td>c</td><td>3</td><td>03/05/2026</td>")


# ---=== La recherche ===---

def test_la_recherche_distingue_le_papier_de_l_electronique():
    """Un scan n'a pas de tableau : le lire rendrait zéro transaction, ce qui se
    lirait comme un rapport vide."""
    lignes = S.parser_recherche(RECHERCHE)
    assert len(lignes) == 2
    assert lignes[0]["papier"] is False
    assert lignes[1]["papier"] is True


def test_la_recherche_reconstruit_l_adresse_complete():
    lignes = S.parser_recherche(RECHERCHE)
    assert lignes[0]["url"] == S.RACINE + "/search/view/ptr/abc-123/"


def test_la_recherche_retire_le_balisage_des_noms():
    """Le service enveloppe certaines cellules dans des liens : les garder
    mettrait du HTML dans la colonne « nom »."""
    lignes = S.parser_recherche(RECHERCHE)
    assert lignes[0]["nom"] == "Jane Doe"
    assert "<" not in lignes[0]["nom"]


def test_une_recherche_vide_ne_plante_pas():
    assert S.parser_recherche({}) == []
    assert S.parser_recherche({"data": []}) == []
    # Une ligne tronquée est ignorée plutôt que de faire échouer le lot.
    assert S.parser_recherche({"data": [["Jane"]]}) == []


# ---=== Le rapport ===---

def test_les_trois_transactions_sont_lues():
    ops = S.parser_rapport(RAPPORT, url="u")
    assert len(ops) == 3
    assert [o["operation"] for o in ops] == ["achat", "vente partielle", "vente"]
    assert [o["sens"] for o in ops] == [1, -1, -1]


def test_les_colonnes_sont_reperees_par_leur_titre():
    """Une colonne insérée en tête décalerait toute la lecture par position, et
    les montants se retrouveraient dans la colonne du type — sans erreur."""
    normal = S.parser_rapport(RAPPORT, url="u")
    decale = S.parser_rapport(RAPPORT_DECALE, url="u")
    assert len(decale) == 3
    for a, b in zip(normal, decale):
        assert a["symbole"] == b["symbole"]
        assert a["montant_min"] == b["montant_min"]
        assert a["operation"] == b["operation"]


def test_un_titre_non_cote_n_a_pas_de_symbole():
    """« -- » remplit la case : le garder ferait un faux symbole."""
    ops = S.parser_rapport(RAPPORT, url="u")
    assert ops[1]["symbole"] is None
    assert ops[1]["actif"] == "Municipal Bond Fund"
    assert ops[0]["symbole"] == "NVDA"


def test_le_detenteur_est_traduit():
    ops = S.parser_rapport(RAPPORT, url="u")
    assert [o["detenteur"] for o in ops] == ["conjoint", "l'élu", "compte joint"]


def test_un_plafond_ouvert_n_est_pas_invente():
    """« Over $50,000,000 » n'a pas de plafond : en fabriquer un fausserait toute
    somme."""
    ops = S.parser_rapport(RAPPORT, url="u")
    assert ops[2]["montant_min"] == 50_000_000
    assert ops[2]["montant_max"] is None


def test_le_commentaire_vide_ne_devient_pas_un_tiret():
    ops = S.parser_rapport(RAPPORT, url="u")
    assert ops[0]["description"] == "--"     # tel que servi, pas inventé
    assert ops[1]["description"] == "Partial redemption"


def test_le_nom_se_lit_dans_le_titre_a_defaut():
    """Quand la recherche ne l'a pas fourni, le rapport le porte."""
    ops = S.parser_rapport(RAPPORT, url="u")
    assert "Jane Doe" in ops[0]["nom"]
    # Et un nom explicite l'emporte.
    ops = S.parser_rapport(RAPPORT, url="u", nom="Jane Doe")
    assert ops[0]["nom"] == "Jane Doe"


def test_un_html_sans_tableau_de_transactions_rend_rien():
    """Rendre des lignes vides ferait passer un rapport illisible pour un rapport
    sans opération."""
    assert S.parser_rapport("<html><body>rien</body></html>") == []
    assert S.parser_rapport("") == []
    # Un tableau qui n'est pas celui des transactions non plus.
    autre = "<table><tr><th>Nom</th><th>Valeur</th></tr><tr><td>a</td><td>b</td></tr></table>"
    assert S.parser_rapport(autre) == []


# ---=== La trame ===---

def test_la_trame_porte_les_memes_colonnes_que_la_chambre():
    """C'est ce qui permet de concaténer les deux chambres sans retouche."""
    import congres_data as C
    assert S.COLONNES == C.COLONNES


def test_les_deux_chambres_se_concatenent():
    import congres_data as C
    senat = S.en_trame(S.parser_rapport(RAPPORT, url="u"))
    chambre = C.en_trame([])
    ensemble = pd.concat([senat, chambre], ignore_index=True)
    assert len(ensemble) == 3
    assert set(ensemble.chambre.dropna()) == {"senat"}


def test_le_senat_n_a_pas_de_delai_de_notification():
    """Il ne publie pas de date distincte : la recopier inventerait un délai nul."""
    df = S.en_trame(S.parser_rapport(RAPPORT, url="u"))
    assert df.date_notification.isna().all()
    assert df.delai_jours.isna().all()


def test_le_milieu_n_existe_que_si_les_deux_bornes_existent():
    df = S.en_trame(S.parser_rapport(RAPPORT, url="u"))
    ferme = df[df.symbole == "NVDA"].iloc[0]
    assert ferme.montant_milieu == pytest.approx((15_001 + 50_000) / 2)
    ouvert = df[df.symbole == "TSLA"].iloc[0]
    assert pd.isna(ouvert.montant_milieu)


def test_une_trame_vide_reste_utilisable():
    df = S.en_trame([])
    assert df.empty and "symbole" in df.columns


# ---=== La pagination ===---

class _ServiceFactice:
    """Imite le service : cent lignes au maximum par appel, quoi qu'on demande.

    C'est le comportement qui rendait l'historique invisible — demander mille
    rapports en rendait cent, toujours les mêmes.
    """

    def __init__(self, total):
        self.total = total
        self.appels = []

    def post(self, url, **kw):
        donnees = kw["data"]
        debut, taille = int(donnees["start"]), int(donnees["length"])
        self.appels.append((debut, taille))
        combien = max(0, min(taille, S.PAGE, self.total - debut))
        lignes = [
            ["Jane", f"Doe{debut + i}", "Senator",
             f'<a href="/search/view/ptr/id-{debut + i}/">Rapport</a>', "03/15/2026"]
            for i in range(combien)
        ]
        return _Reponse({"data": lignes, "recordsTotal": self.total})


class _Reponse:
    status_code = 200

    def __init__(self, charge):
        self._charge = charge

    def json(self):
        return self._charge

    def raise_for_status(self):
        pass


def test_la_pagination_va_chercher_au_dela_de_la_premiere_page():
    """Le service plafonne à cent lignes : sans pagination, on ne voit que les
    cent derniers rapports sur les deux mille quatre cents de l'archive."""
    service = _ServiceFactice(total=2417)
    rapports = S.chercher_rapports(service, "jeton", limite=250, pause=0)
    assert len(rapports) == 250
    # Trois appels : 100, 100, puis 50 — et jamais deux fois le même début.
    assert [d for d, _ in service.appels] == [0, 100, 200]
    assert len({r["url"] for r in rapports}) == 250


def test_la_pagination_s_arrete_au_total_annonce():
    """Continuer au-delà ferait des appels qui ne rendent rien."""
    service = _ServiceFactice(total=120)
    rapports = S.chercher_rapports(service, "jeton", limite=500, pause=0)
    assert len(rapports) == 120
    assert len(service.appels) == 2


def test_une_page_vide_arrete_la_course():
    """Le service ne dit pas toujours qu'il a fini : une page vide est le seul
    signal fiable."""
    service = _ServiceFactice(total=0)
    service.total = 0
    assert S.chercher_rapports(service, "jeton", limite=300, pause=0) == []
    assert len(service.appels) == 1


def test_la_limite_est_respectee_a_la_ligne_pres():
    """Rendre plus que demandé ferait payer des téléchargements inutiles."""
    service = _ServiceFactice(total=2417)
    assert len(S.chercher_rapports(service, "jeton", limite=42, pause=0)) == 42
    assert service.appels == [(0, 42)]


# ---=== L'accès ===---

def test_les_entetes_sont_complets(monkeypatch):
    """Un User-Agent seul ne suffit pas : mesuré, la requête amputée de ses
    autres en-têtes reçoit un 403 même avec celui d'un navigateur."""
    monkeypatch.delenv(S.VARIABLE_UA, raising=False)
    envoyes = S.entetes()
    for attendu in ("User-Agent", "Accept", "Accept-Language", "Sec-Fetch-Mode"):
        assert attendu in envoyes, attendu


def test_la_variable_remplace_le_seul_user_agent(monkeypatch):
    """Le reste est ce qui fait qu'une requête est complète : le laisser
    remplacer ferait échouer la connexion sans dire pourquoi."""
    monkeypatch.setenv(S.VARIABLE_UA, "Autre client")
    envoyes = S.entetes()
    assert envoyes["User-Agent"] == "Autre client"
    assert envoyes["Accept-Language"] == S.ENTETES_HTTP["Accept-Language"]


def test_un_refus_dit_ou_est_le_probleme(monkeypatch):
    """403 n'est pas une panne réseau : le service n'accepte que ce qui se
    présente comme un navigateur, et insister ne sert à rien."""
    class Reponse:
        status_code = 403

        def raise_for_status(self):
            raise AssertionError("ne doit pas être atteint")

    with pytest.raises(S.AccesRefuse, match="navigateur"):
        S._verifier(Reponse())
