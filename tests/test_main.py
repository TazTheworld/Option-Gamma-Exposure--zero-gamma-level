"""main.py : les décisions qu'il prend avant de calculer quoi que ce soit.

Le lecteur ne va plus chercher de données — il ouvre un relevé écrit par
ib_collector.py. Ce qui reste à décider tient en trois choses : quel fichier
lire, quelles combinaisons de drapeaux refuser, et par quel multiplicateur
compter. La troisième est la seule qui puisse produire un chiffre faux sans le
dire, d'où sa place ici.
"""

import pytest

import main
from black76 import taille_contrat


def _args(*argv):
    return main.construire_parser().parse_args(list(argv))


# ---=== Le multiplicateur ===---

def test_le_produit_decide_du_multiplicateur():
    """Le NQ est le E-mini, à x20. Le confondre avec les x100 des actions
    donnerait un GEX cinq fois trop grand — et cinq fois trop grand reste un
    nombre plausible, donc rien ne le signalerait."""
    assert main.multiplicateur(_args("NQ")) == 20
    assert main.multiplicateur(_args("ES")) == 50


def test_le_multiplicateur_ne_depend_plus_du_drapeau_de_source():
    """La régression que ce test ferme : le multiplicateur se lisait autrefois sur
    `args.cme or args.databento`, donc un relevé lu depuis le disque — le seul cas
    qui reste — retombait sur les x100 des actions."""
    assert main.multiplicateur(_args("NQ", "--suivre")) == 20
    assert main.multiplicateur(_args("NQ", "--replay", "x.parquet")) == 20


def test_le_multiplicateur_explicite_prime():
    assert main.multiplicateur(_args("NQ", "--contract-size", "2")) == 2


def test_un_produit_inconnu_est_refuse_pas_devine():
    """Un défaut silencieux ferait aboutir le calcul sur un chiffre faux."""
    with pytest.raises(ValueError, match="Multiplicateur inconnu"):
        main.multiplicateur(_args("TSLA"))
    with pytest.raises(ValueError, match="NQ"):
        taille_contrat("ZZZZ")


# ---=== La source ===---

def test_sans_drapeau_la_source_est_le_releve_courant():
    """Il n'y a plus qu'une source : exiger --suivre pour la nommer ferait un
    drapeau obligatoire, donc inutile."""
    attendu = main.snapshots.courant("NQ", main.snapshots.DOSSIER)
    assert main.source_relecture(_args("NQ")) == attendu
    assert main.source_relecture(_args("NQ", "--suivre")) == attendu


def test_replay_prime_sur_le_courant():
    assert main.source_relecture(_args("NQ", "--replay", "a.parquet")) == "a.parquet"


def test_replay_et_suivre_s_excluent():
    with pytest.raises(ValueError, match="s'excluent"):
        main.verifier_exclusions(_args("NQ", "--replay", "a.parquet", "--suivre"))


def test_watch_reste_interdit_sur_une_archive_mais_permis_sur_le_courant():
    """Une archive ne bouge plus ; le courant, lui, est réécrit toutes les quinze
    secondes par le collecteur."""
    with pytest.raises(ValueError, match="ne bouge plus"):
        main.verifier_exclusions(_args("NQ", "--replay", "a.parquet", "--watch", "30"))
    main.verifier_exclusions(_args("NQ", "--suivre", "--watch", "30"))


def test_un_releve_absent_nomme_le_programme_qui_l_ecrit(tmp_path):
    """Sans ça, l'utilisateur lit une trace de fichier introuvable sans savoir
    quel programme aurait dû l'écrire."""
    with pytest.raises(OSError, match="ib_collector.py NQ"):
        main.charger(_args("NQ", "--dir", str(tmp_path)))


# ---=== Les durées de --watch ===---

@pytest.mark.parametrize("texte,secondes",
                         [("6h", 21600), ("90m", 5400), ("300", 300), ("1.5h", 5400)])
def test_parsing_des_durees(texte, secondes):
    assert main.duree(texte) == secondes
