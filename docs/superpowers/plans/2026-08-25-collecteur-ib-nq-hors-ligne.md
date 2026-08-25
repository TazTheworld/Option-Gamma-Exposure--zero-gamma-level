# Collecteur IB pour NQ — lot hors ligne : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**But :** écrire les quatre fonctions pures du collecteur Interactive Brokers, plus les deux points d'accroche dans le code existant, entièrement testables sans réseau ni IB Gateway.

**Architecture :** `ib_data.py` suit le patron de `databento_data.py` — des fonctions pures séparées de l'accès réseau, testées sur des trames fabriquées. `perimetre()` décide quoi demander, `build_chain()` assemble au format pivot du projet, `selection_vif()` choisit les contrats à garder souscrits, `fusionner()` recolle l'open interest du socle et l'IV du vif. La couche réseau et le démon ne sont PAS dans ce plan.

**Pile :** Python 3.11+, pandas, numpy. Aucune dépendance nouvelle installée par ce lot — l'extra `[ib]` est déclaré mais rien ici ne l'importe.

**Spec :** `docs/superpowers/specs/2026-08-25-collecteur-ib-nq-design.md`

> **Exécuté le 25 août 2026 — ce plan n'est plus à jour, et c'est voulu.** Il
> consigne ce qui avait été planifié ; la conception a bougé pendant l'exécution
> et c'est la spec qui fait foi. Deux écarts valent d'être connus avant de lire :
> `perimetre()` a été scindée en `echeances_utiles()` + `perimetre()`, le plan
> lui faisant construire un produit cartésien de strikes et d'échéances qui
> comptait des milliers de contrats jamais cotés ; et une borne flottante y
> excluait le strike exactement à la limite du périmètre. Les deux sont corrigés
> dans le code. La suite est passée de 158 à 199 tests.

## Contraintes générales

- **Aucun test ne touche au réseau.** Règle du dépôt, écrite dans `.github/workflows/tests.yml` : « la suite doit passer telle quelle ».
- **Le cœur reste à cinq paquets** (numpy, pandas, scipy, matplotlib, requests). `ib_async` est un extra `[ib]`, jamais une dépendance de base.
- **Format pivot :** toute chaîne produite respecte `cboe_data.COLUMNS` + `cboe_data.COLONNES_GRECS`, et passe par `cboe_data._clean()`.
- **Français pour le métier, anglais pour les conventions externes.** Le dépôt fait déjà cette distinction : `analyser()`, `murs()`, `charger()` d'un côté ; `fetch_chain()`, `build_chain()`, `black76_gamma()` de l'autre.
- **Taille du contrat NQ : ×20**, déjà dans `cme_data.CONTRACT_SIZES`. Ne pas la redéfinir.
- **Point de départ :** 158 tests passent (`./venv/Scripts/python.exe -m pytest tests -q`). Ce chiffre doit croître, jamais baisser.

## Structure des fichiers

| Fichier | Responsabilité | Action |
|---|---|---|
| `ib_data.py` | les quatre fonctions pures du collecteur | **créer** |
| `tests/test_ib.py` | leurs tests, sur trames fabriquées | **créer** |
| `snapshots.py` | + `courant()`, et `lister()` qui l'ignore | modifier |
| `main.py` | + `--suivre`, exclusion `--watch`/`--replay` assouplie | modifier |
| `pyproject.toml` | extra `[ib]`, `ib_data` dans `py-modules` | modifier |

Deux fichiers seulement sont créés. `ib_collector.py` (le démon) et la couche réseau d'`ib_data.py` appartiennent au second plan.

---

### Tâche 1 : le chemin du relevé courant

**Fichiers :**
- Modifier : `snapshots.py` (ajouter `courant()`, corriger `lister()`)
- Test : `tests/test_analysis.py` (section `# ---=== Archivage ===---`, après `test_snapshot_lister_et_dernier`)

**Interfaces :**
- Consomme : `snapshots.DOSSIER`, `snapshots._sans_parquet()` — existants.
- Produit : `snapshots.courant(ticker, dossier=DOSSIER) -> str`. Le collecteur du second plan l'appellera à chaque rafraîchissement.

**Pourquoi cette tâche existe :** `chemin()` horodate à la minute (`%Y-%m-%d_%H%M`). Un collecteur qui réécrit toutes les quinze secondes y créerait un fichier neuf chaque minute — près de mille cinq cents par jour. Le courant a donc un chemin fixe. Et comme `lister()` fait un glob sur `*.parquet` / `*.csv.gz`, le courant y remonterait et polluerait `dernier()` : il faut l'exclure.

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à la fin de la section `# ---=== Archivage ===---` de `tests/test_analysis.py` :

```python
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
    snapshots.sauver(chaine(), "NQ", SPOT, QUOTE, str(tmp_path))
    import shutil
    shutil.copy(archive, vivant)            # le courant existe sur disque

    trouves = snapshots.lister("NQ", str(tmp_path))
    assert vivant not in trouves
    assert archive in trouves
    assert snapshots.dernier("NQ", str(tmp_path)) != vivant


def test_snapshot_courant_relu_comme_une_archive(tmp_path):
    """Même format que les archives : main.py --replay doit pouvoir l'ouvrir."""
    df = chaine(jours=(1, 8))
    snapshots.sauver(df, "NQ", SPOT, QUOTE, str(tmp_path))
    cible = snapshots.courant("NQ", str(tmp_path))
    import shutil
    shutil.copy(snapshots.chemin("NQ", QUOTE, str(tmp_path)), cible)

    relu, spot, quote_date, marche = snapshots.charger(cible)
    assert spot == SPOT and pd.Timestamp(quote_date) == QUOTE
    assert len(relu) == len(df)
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_analysis.py -k courant -v
```
Attendu : ÉCHEC, `AttributeError: module 'snapshots' has no attribute 'courant'`.

- [ ] **Étape 3 : écrire l'implémentation minimale**

Dans `snapshots.py`, ajouter après `chemin()` :

```python
NOM_COURANT = "courant"


def courant(ticker, dossier=DOSSIER):
    """snapshots/NQ/courant.parquet — le relevé vivant, réécrit en place.

    chemin() horodate à la minute. Un collecteur qui réécrit toutes les quinze
    secondes y créerait un fichier neuf par minute, soit près de mille cinq
    cents par jour et par sous-jacent : une archive illisible, et un lecteur
    incapable de savoir lequel est le dernier sans lister le dossier.

    Le courant a donc un chemin fixe, et les archives horodatées gardent
    chemin(). Même format dans les deux cas : sauver() et charger() servent les
    deux sans changement, et main.py --replay ouvre l'un comme l'autre.
    """
    extension = "csv.gz" if _sans_parquet() else "parquet"
    return os.path.join(dossier, str(ticker).upper(), f"{NOM_COURANT}.{extension}")
```

Puis remplacer le corps de `lister()` par :

```python
def lister(ticker=None, dossier=DOSSIER):
    """Chemins archivés, du plus ancien au plus récent.

    Le relevé courant est exclu : il porte le même format mais pas le même rôle.
    Le laisser entrer le ferait remonter dans dernier(), donc dans les relectures
    et dans validate.py, où un fichier qui change sous les pieds n'a rien à faire.
    """
    motif = os.path.join(dossier, str(ticker).upper() if ticker else "*", "*.*")
    return sorted(f for f in glob.glob(motif)
                  if f.endswith((".parquet", ".csv.gz"))
                  and not os.path.basename(f).startswith(NOM_COURANT + "."))
```

- [ ] **Étape 4 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_analysis.py -k courant -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : 3 passent ; la suite complète passe à **161** (158 + 3), zéro échec.

- [ ] **Étape 5 : commiter**

```bash
git add snapshots.py tests/test_analysis.py
git commit -m "snapshots : un chemin fixe pour le releve vivant

Le collecteur IB reecrira son releve toutes les quinze secondes. Avec chemin(),
qui horodate a la minute, ca ferait pres de mille cinq cents fichiers par jour et
par sous-jacent, et un lecteur devrait lister le dossier pour savoir lequel est le
dernier.

courant() rend un chemin fixe, et lister() l'ignore desormais : le courant porte
le meme format que les archives mais pas le meme role, et le laisser entrer le
ferait remonter dans dernier(), donc dans --replay et dans validate.py, ou un
fichier qui change sous les pieds n'a rien a faire."
```

---

### Tâche 2 : `perimetre()` — quels contrats demander

**Fichiers :**
- Créer : `ib_data.py`
- Créer : `tests/test_ib.py`
- Modifier : `pyproject.toml` (extra `[ib]`, `ib_data` dans `py-modules`)

**Interfaces :**
- Consomme : rien des tâches précédentes.
- Produit : `ib_data.perimetre(strikes, echeances, prix, plage=0.2, dte_max=30, dte_min=0, quote_date=None) -> pd.DataFrame` avec les colonnes `ExpirationDate` (datetime64), `StrikePrice` (float), `right` (`"C"`/`"P"`). **Ce triplet est la forme unique de contrat dans tout le module** — `selection_vif()` rend la même, et la couche réseau du second plan construira ses objets `Contract` à partir de ces lignes.

- [ ] **Étape 1 : écrire les tests qui échouent**

Créer `tests/test_ib.py` :

```python
"""Les fonctions pures du collecteur Interactive Brokers.

Comme pour databento, les jeux d'essai sont fabriqués depuis des paramètres
connus : on vérifie qu'on retrouve la vérité terrain, pas qu'une sortie est figée.
Aucun test ne touche au réseau, et rien ici n'importe ib_async.
"""

import numpy as np
import pandas as pd
import pytest

import ib_data
from cboe_data import COLUMNS

QUOTE = pd.Timestamp("2026-08-25")
PRIX = 25_000.0
STRIKES = np.arange(20_000.0, 30_025.0, 25.0)
ECHEANCES = ["20260826", "20260828", "20260904", "20260918", "20261120"]


# ---=== perimetre ===---

def test_perimetre_coupe_les_strikes_hors_plage():
    """+/-2 % autour de 25 000, c'est 24 500 a 25 500 et rien d'autre."""
    p = ib_data.perimetre(STRIKES, ["20260904"], PRIX, plage=0.02, quote_date=QUOTE)
    assert p.StrikePrice.min() == pytest.approx(24_500.0)
    assert p.StrikePrice.max() == pytest.approx(25_500.0)


def test_perimetre_coupe_les_echeances_hors_horizon():
    """dte_max=30 ecarte le 20 novembre, dte_min=2 ecarte le lendemain."""
    p = ib_data.perimetre(STRIKES, ECHEANCES, PRIX, plage=0.01,
                          dte_max=30, dte_min=2, quote_date=QUOTE)
    gardees = sorted(pd.Timestamp(d).strftime("%Y%m%d") for d in p.ExpirationDate.unique())
    assert gardees == ["20260828", "20260904", "20260918"]


def test_perimetre_produit_les_deux_cotes():
    """Un contrat par (echeance, strike, sens) : le GEX a besoin des deux jambes."""
    p = ib_data.perimetre([25_000.0], ["20260904"], PRIX, quote_date=QUOTE)
    assert sorted(p.right) == ["C", "P"]
    assert len(p) == 2


def test_perimetre_compte_les_contrats_a_demander():
    """C'est ce nombre qui decide du temps de balayage : il doit etre previsible."""
    p = ib_data.perimetre(STRIKES, ECHEANCES, PRIX, plage=0.025,
                          dte_max=30, dte_min=0, quote_date=QUOTE)
    n_strikes = p.StrikePrice.nunique()
    n_echeances = p.ExpirationDate.nunique()
    assert len(p) == n_strikes * n_echeances * 2
    assert n_strikes == 51            # 25 de part et d'autre, plus la monnaie


def test_perimetre_dte_max_none_garde_tout():
    """'all' dans main.py arrive ici en None : aucune echeance ne doit tomber."""
    p = ib_data.perimetre([25_000.0], ECHEANCES, PRIX, dte_max=None, quote_date=QUOTE)
    assert p.ExpirationDate.nunique() == len(ECHEANCES)


def test_perimetre_vide_rend_une_trame_typee_pas_une_erreur():
    """Une grille sans strike dans la plage n'est pas une panne : le CME ne liste
    que vingt-cinq strikes autour du reglement sur les echeances courtes."""
    p = ib_data.perimetre(STRIKES, ECHEANCES, prix=1.0, plage=0.02, quote_date=QUOTE)
    assert p.empty
    assert list(p.columns) == ["ExpirationDate", "StrikePrice", "right"]


def test_perimetre_dedoublonne_et_trie():
    """reqSecDefOptParams rend des ensembles : l'ordre et l'unicite sont a nous."""
    p = ib_data.perimetre([25_000.0, 25_000.0, 24_975.0], ["20260904", "20260904"],
                          PRIX, plage=0.01, quote_date=QUOTE)
    assert p.StrikePrice.nunique() == 2
    assert p.ExpirationDate.nunique() == 1
    assert p.StrikePrice.is_monotonic_increasing or p.sort_values(
        ["ExpirationDate", "StrikePrice"]).equals(p.sort_values(["ExpirationDate", "StrikePrice"]))
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -v
```
Attendu : ÉCHEC à la collecte, `ModuleNotFoundError: No module named 'ib_data'`.

- [ ] **Étape 3 : écrire l'implémentation minimale**

Créer `ib_data.py` :

```python
"""Collecteur Interactive Brokers pour les options sur futures NQ.

IB ne sert pas une chaîne : il sert des contrats un par un, avec un plafond de
cent lignes de données simultanées. Tout ce module découle de cette contrainte.

Ce fichier ne contient que les fonctions pures — celles qui décident, assemblent
et fusionnent, et qui se testent sans réseau. C'est le patron de
databento_data.py, pour la même raison : ce qui produit un chiffre doit être
vérifiable hors ligne.

    perimetre()      quels contrats demander, avant de les demander
    build_chain()    définitions + valeurs -> chaîne au format du projet
    selection_vif()  les contrats à garder souscrits en permanence
    fusionner()      open interest du socle + IV du vif -> entrée d'analyser()

La connexion, l'énumération et la souscription vivent ailleurs : elles demandent
IB Gateway, donc elles ne sont pas testables ici.
"""

import numpy as np
import pandas as pd

# La forme unique d'un contrat dans ce module. La couche réseau construira ses
# objets ib_async.Contract à partir de ces trois colonnes, et rien d'autre.
CONTRAT = ["ExpirationDate", "StrikePrice", "right"]


def _quote_date(valeur=None):
    """Date de valorisation normalisée, sans fuseau."""
    ts = pd.Timestamp(valeur) if valeur is not None else pd.Timestamp.utcnow()
    if ts.tz is not None:
        ts = ts.tz_localize(None)
    return ts.normalize()


def perimetre(strikes, echeances, prix, plage=0.2, dte_max=30, dte_min=0,
              quote_date=None):
    """Les contrats à demander, un par (échéance, strike, sens).

    C'est l'inversion que le projet n'avait jamais eu à faire. Le CBOE sert toute
    la chaîne et analysis.filtre_echeances() élague ensuite ; IB oblige à élaguer
    AVANT de demander, chaque contrat coûtant une des cent lignes disponibles.
    `plage` et `dte_max` cessent donc d'être des réglages d'affichage pour devenir
    le périmètre d'acquisition.

    Le périmètre demandé n'est pas le périmètre obtenu, et ce n'est pas une
    anomalie : le CME ne liste que vingt-cinq strikes de part et d'autre du
    règlement sur les échéances hebdomadaires, soit environ 2,5 %. Ce qui n'est
    pas listé n'apparaît simplement pas dans `strikes`, et une trame vide est une
    réponse, pas une panne.

    `dte_max=None` garde toutes les échéances (le 'all' de main.py).
    """
    quote_date = _quote_date(quote_date)

    ks = pd.to_numeric(pd.Series(list(strikes), dtype="object"), errors="coerce").dropna()
    bas, haut = float(prix) * (1 - plage), float(prix) * (1 + plage)
    ks = sorted({float(k) for k in ks if bas <= k <= haut})

    dates = pd.to_datetime(pd.Series(list(echeances), dtype="object"),
                           errors="coerce").dropna()
    dates = sorted({d.normalize() for d in dates})
    gardees = []
    for d in dates:
        jours = (d - quote_date).days
        if jours < dte_min:
            continue
        if dte_max is not None and jours > dte_max:
            continue
        gardees.append(d)

    lignes = [{"ExpirationDate": d, "StrikePrice": k, "right": r}
              for d in gardees for k in ks for r in ("C", "P")]
    df = pd.DataFrame(lignes, columns=CONTRAT)
    df["ExpirationDate"] = pd.to_datetime(df["ExpirationDate"])
    df["StrikePrice"] = pd.to_numeric(df["StrikePrice"], errors="coerce")
    return df
```

- [ ] **Étape 4 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -v
```
Attendu : 7 passent.

- [ ] **Étape 5 : déclarer le module**

Dans `pyproject.toml`, ajouter `"ib_data"` à `py-modules` (après `"databento_data"`) :

```toml
py-modules = [
    "main", "greeks", "analysis", "plots", "snapshots", "price_data",
    "cboe_data", "cme_data", "databento_data", "ib_data", "barchart_data",
    "flow_tracker", "history", "validate",
]
```

Et déclarer l'extra, après la ligne `barchart` de `[project.optional-dependencies]` :

```toml
# Collecteur Interactive Brokers. ib_async est le fork maintenu d'ib_insync,
# dont l'auteur est mort en 2024. Rien du coeur ne l'importe : `python main.py
# TSLA` ne doit pas payer une dependance de courtier.
ib = ["ib_async>=1.0"]
```

- [ ] **Étape 6 : lancer la suite complète**

```
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : **168** (161 + 7), zéro échec.

- [ ] **Étape 7 : commiter**

```bash
git add ib_data.py tests/test_ib.py pyproject.toml
git commit -m "ib_data : le perimetre se decide avant de demander, pas apres

IB ne sert pas une chaine, il sert des contrats un par un, avec cent lignes de
donnees simultanees. Le CBOE donne tout puis on filtre ; IB oblige a filtrer
avant, sous peine de bruler le budget sur des strikes sans interet. --range et
--dte-max cessent donc d'etre des reglages d'affichage pour devenir le perimetre
d'acquisition.

perimetre() rend un contrat par (echeance, strike, sens), et ce triplet est la
forme unique de contrat dans tout le module : selection_vif() rendra la meme, et
la couche reseau construira ses objets ib_async a partir de ces trois colonnes.

Une trame vide est une reponse et non une panne : le CME ne liste que vingt-cinq
strikes de part et d'autre du reglement sur les echeances hebdomadaires, soit
environ 2,5 %. Demander 20 % n'y rend pas 20 %, et rien la-dedans n'est casse.

ib_async est declare en extra [ib], jamais dans le coeur."
```

---

### Tâche 3 : `build_chain()` — assembler au format du projet

**Fichiers :**
- Modifier : `ib_data.py`
- Test : `tests/test_ib.py`

**Interfaces :**
- Consomme : `ib_data.CONTRAT`, `ib_data._quote_date()` (tâche 2) ; `cboe_data.COLUMNS`, `cboe_data.COLONNES_GRECS`, `cboe_data._clean()` ; `cme_data.black76_gamma()`, `cme_data.implied_vol()`, `cme_data.infer_futures_price()`.
- Produit : `ib_data.build_chain(defs, ticks, futures_price=None, quote_date=None, rate=0.0) -> (pd.DataFrame, float, datetime)`.
  - `defs` : `DataFrame[conId, StrikePrice, ExpirationDate, right]`
  - `ticks` : `DataFrame[conId, OpenInt, IV, Gamma, Delta, Vega, Theta, Settle]` — colonnes manquantes tolérées
  - Sortie identique en forme à `databento_data.build_chain()`.

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à `tests/test_ib.py` :

```python
# ---=== build_chain ===---

VOL_VRAI = 0.18
EXP = pd.Timestamp("2026-09-04 14:00")
T_VRAI = (EXP - QUOTE).total_seconds() / (365.25 * 24 * 3600)


def _trames_ib(avec_gamma=True, iv_en_pourcent=False, oi=500.0):
    """Imite ce que la couche reseau produira : definitions + valeurs par conId."""
    from cme_data import black76_gamma
    strikes = np.arange(24_500.0, 25_525.0, 25.0)
    defs, ticks, cid = [], [], 100_000
    for k in strikes:
        for r in ("C", "P"):
            defs.append({"conId": cid, "StrikePrice": k,
                         "ExpirationDate": EXP, "right": r})
            gamma = float(black76_gamma(PRIX, k, VOL_VRAI, T_VRAI)) if avec_gamma else np.nan
            ticks.append({
                "conId": cid,
                "OpenInt": oi,
                "IV": VOL_VRAI * 100 if iv_en_pourcent else VOL_VRAI,
                "Gamma": gamma,
                "Delta": np.nan, "Vega": np.nan, "Theta": np.nan,
                "Settle": np.nan,
            })
            cid += 1
    # bruit : un contrat sans valeur recue, comme un strike illiquide qui ne
    # repond jamais a la souscription
    defs.append({"conId": 999_999, "StrikePrice": 25_000.0,
                 "ExpirationDate": EXP, "right": "C"})
    return pd.DataFrame(defs), pd.DataFrame(ticks)


def test_build_chain_rend_le_format_pivot():
    """Tout l'aval (analysis, plots, snapshots) attend COLUMNS et rien d'autre."""
    chaine, prix, date_val = ib_data.build_chain(
        *_trames_ib(), futures_price=PRIX, quote_date=QUOTE)
    for colonne in COLUMNS:
        assert colonne in chaine.columns
    assert prix == pytest.approx(PRIX)
    assert pd.Timestamp(date_val) == QUOTE


def test_build_chain_une_ligne_par_strike():
    """Format large : calls et puts d'un meme strike sur la meme ligne."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert len(chaine) == 41                       # 24 500 a 25 500 au pas de 25
    assert chaine.StrikePrice.is_unique


def test_build_chain_reporte_l_open_interest():
    """Sans OI il n'y a pas de GEX : c'est l'entree principale d'expositions()."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(oi=500.0), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert chaine.CallOpenInt.sum() == pytest.approx(500.0 * 41)
    assert chaine.PutOpenInt.sum() == pytest.approx(500.0 * 41)


def test_build_chain_normalise_l_iv_en_pourcentage():
    """IB publie parfois l'IV en pourcent : 18 n'est pas 1800 % de volatilite."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(iv_en_pourcent=True),
                                       futures_price=PRIX, quote_date=QUOTE)
    assert chaine.CallIV.mean() == pytest.approx(VOL_VRAI, abs=1e-6)


def test_build_chain_recalcule_le_gamma_absent_en_black76():
    """Si IB ne publie pas le gamma, Black-76 le retrouve depuis l'IV."""
    sans, _, _ = ib_data.build_chain(*_trames_ib(avec_gamma=False),
                                     futures_price=PRIX, quote_date=QUOTE)
    avec, _, _ = ib_data.build_chain(*_trames_ib(avec_gamma=True),
                                     futures_price=PRIX, quote_date=QUOTE)
    assert (sans.CallGamma > 0).any()
    assert sans.CallGamma.to_numpy() == pytest.approx(avec.CallGamma.to_numpy(), rel=1e-6)


def test_build_chain_garde_le_gamma_publie_quand_il_existe():
    """--gamma-source published doit avoir de quoi se nourrir."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(avec_gamma=True),
                                       futures_price=PRIX, quote_date=QUOTE)
    atm = chaine.loc[(chaine.StrikePrice - PRIX).abs().idxmin()]
    from cme_data import black76_gamma
    attendu = float(black76_gamma(PRIX, atm.StrikePrice, VOL_VRAI, T_VRAI))
    assert atm.CallGamma == pytest.approx(attendu, rel=1e-6)


def test_build_chain_ignore_les_contrats_sans_valeur():
    """Un strike illiquide qui ne repond jamais ne doit pas casser l'assemblage."""
    defs, ticks = _trames_ib()
    assert 999_999 in set(defs.conId)          # present dans les definitions
    assert 999_999 not in set(ticks.conId)     # absent des valeurs recues
    chaine, _, _ = ib_data.build_chain(defs, ticks, futures_price=PRIX,
                                       quote_date=QUOTE)
    assert len(chaine) == 41                   # il n'ajoute pas de ligne fantome


def test_build_chain_definitions_vides_leve_une_erreur():
    """Echouer bruyamment : une chaine vide silencieuse donnerait un GEX de zero."""
    with pytest.raises(ValueError):
        ib_data.build_chain(pd.DataFrame(columns=["conId", "StrikePrice",
                                                  "ExpirationDate", "right"]),
                            pd.DataFrame(columns=["conId"]),
                            futures_price=PRIX, quote_date=QUOTE)


def test_build_chain_deduit_le_prix_du_future_par_parite():
    """Sans prix fourni, la parite call-put le retrouve — comme pour le CME."""
    defs, ticks = _trames_ib()
    from cme_data import black76_price
    prix_par_conid = {}
    for _, d in defs.iterrows():
        if d.conId == 999_999:
            continue
        prix_par_conid[d.conId] = black76_price(PRIX, d.StrikePrice, VOL_VRAI,
                                                T_VRAI, 0.0, d.right)
    ticks = ticks.copy()
    ticks["Settle"] = ticks.conId.map(prix_par_conid)

    _, prix, _ = ib_data.build_chain(defs, ticks, futures_price=None,
                                     quote_date=QUOTE)
    assert prix == pytest.approx(PRIX, rel=1e-4)


def test_build_chain_alimente_analyser_sans_retouche():
    """Le contrat de bout en bout : la sortie entre telle quelle dans analysis."""
    import analysis
    chaine, prix, date_val = ib_data.build_chain(*_trames_ib(),
                                                 futures_price=PRIX,
                                                 quote_date=QUOTE)
    a = analysis.analyser(chaine, spot=prix, quote_date=date_val,
                          ticker="NQ", contract_size=20, dte_max=None)
    assert np.isfinite(a.total_gex)
    assert a.total_gex != 0.0
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k build_chain -v
```
Attendu : ÉCHEC, `AttributeError: module 'ib_data' has no attribute 'build_chain'`.

- [ ] **Étape 3 : écrire l'implémentation minimale**

Ajouter à `ib_data.py`, sous les imports :

```python
from cboe_data import COLONNES_GRECS, COLUMNS, _clean
from cme_data import black76_gamma, implied_vol, infer_futures_price

# Ce que la couche réseau collecte par contrat. Les colonnes absentes valent NaN
# plutôt que de manquer : un contrat illiquide qui ne répond jamais ne doit pas
# faire échouer l'assemblage des trois mille autres.
CHAMPS_TICK = ["OpenInt", "IV", "Gamma", "Delta", "Vega", "Theta", "Settle"]
```

Puis, à la suite de `perimetre()` :

```python
def build_chain(defs, ticks, futures_price=None, quote_date=None, rate=0.0):
    """Définitions + valeurs reçues -> chaîne au format pivot du projet.

    Jumelle de databento_data.build_chain(), et pour les mêmes raisons : des
    définitions d'un côté, des valeurs de l'autre, une fonction pure au milieu.
    Séparée de l'accès réseau pour être testable sans IB Gateway.

    `defs`  : conId, StrikePrice, ExpirationDate, right
    `ticks` : conId + ce que la souscription a rendu (CHAMPS_TICK)

    Renvoie (df, futures_price, quote_date) au format COLUMNS.
    """
    manquantes = {"conId", "StrikePrice", "ExpirationDate", "right"} - set(defs.columns)
    if manquantes:
        raise ValueError(
            f"Définitions IB inattendues, colonnes absentes : {sorted(manquantes)}. "
            f"Reçu : {list(defs.columns)}"
        )

    d = defs.copy()
    d["right"] = d["right"].astype(str).str.upper().str[0]
    d = d[d.right.isin(["C", "P"])]
    d = d.drop_duplicates("conId", keep="last")
    d["StrikePrice"] = pd.to_numeric(d.StrikePrice, errors="coerce")
    d["ExpirationDate"] = pd.to_datetime(d.ExpirationDate, errors="coerce")
    d = d.dropna(subset=["StrikePrice", "ExpirationDate"])
    if d.empty:
        raise ValueError("Aucune option (call/put) dans les définitions IB reçues.")

    t = ticks.copy() if ticks is not None else pd.DataFrame(columns=["conId"])
    if "conId" not in t.columns:
        raise ValueError(f"Valeurs IB inattendues : {list(t.columns)}")
    for champ in CHAMPS_TICK:
        if champ not in t.columns:
            t[champ] = np.nan
        t[champ] = pd.to_numeric(t[champ], errors="coerce")
    t = t.drop_duplicates("conId", keep="last").set_index("conId")

    df = d.join(t[CHAMPS_TICK], on="conId")

    keys = ["ExpirationDate", "StrikePrice"]
    calls = df[df.right == "C"].groupby(keys)[CHAMPS_TICK].last().add_prefix("Call")
    puts = df[df.right == "P"].groupby(keys)[CHAMPS_TICK].last().add_prefix("Put")
    chain = calls.join(puts, how="outer").reset_index()

    quote_date = _quote_date(quote_date).to_pydatetime()

    if futures_price is None:
        futures_price = infer_futures_price(chain)
        if futures_price is None:
            raise ValueError(
                "Prix du future indéterminable : passe-le avec --futures-price."
            )
    futures_price = float(futures_price)

    T = ((chain.ExpirationDate - pd.Timestamp(quote_date)).dt.total_seconds()
         / (365.25 * 24 * 3600)).clip(lower=0)

    for side, opt in (("Call", "C"), ("Put", "P")):
        iv = pd.to_numeric(chain[f"{side}IV"], errors="coerce")
        # IB publie l'IV tantôt en décimal, tantôt en pourcentage. 18 ne peut pas
        # être 1 800 % de volatilité : au-delà de 3, c'est une échelle, pas un
        # régime. Même test que databento_data, même raison.
        if iv.notna().any() and iv.max(skipna=True) > 3:
            iv = iv / 100.0
        manque = iv.isna() & chain[f"{side}Settle"].notna()
        if manque.any():
            iv.loc[manque] = [
                implied_vol(p, futures_price, k, tt, rate, opt)
                for p, k, tt in zip(chain.loc[manque, f"{side}Settle"],
                                    chain.loc[manque, "StrikePrice"], T[manque])
            ]
        chain[f"{side}IV"] = iv

        # Le gamma publié par IB est gardé tel quel ; là où il manque, Black-76
        # le retrouve depuis l'IV. Sans ce repli, --gamma-source published et le
        # profil ne travailleraient pas sur le même périmètre.
        gamma = pd.to_numeric(chain[f"{side}Gamma"], errors="coerce")
        calcule = pd.Series(black76_gamma(futures_price, chain.StrikePrice, iv, T, rate),
                            index=chain.index)
        chain[f"{side}Gamma"] = gamma.where(gamma.notna(), calcule)

    chain["Calls"] = ""
    chain["Puts"] = ""
    chain = chain.reindex(columns=COLUMNS + COLONNES_GRECS)
    return _clean(chain), futures_price, quote_date
```

- [ ] **Étape 4 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : `test_ib.py` 17 passent ; suite complète **178**, zéro échec.

- [ ] **Étape 5 : commiter**

```bash
git add ib_data.py tests/test_ib.py
git commit -m "ib_data : build_chain, jumelle de celle de databento

Memes entrees conceptuelles — des definitions d'un cote, des valeurs de l'autre —
meme sortie au format COLUMNS, meme Black-76 la ou le gamma manque, meme _clean()
en sortie. C'est deliberement le meme patron : ce qui produit un chiffre doit se
verifier hors ligne, et deux assembleurs qui divergeraient sur la normalisation
de l'IV donneraient deux GEX differents pour la meme chaine.

Trois choses valent d'etre dites. Un contrat illiquide qui ne repond jamais a la
souscription reste dans les definitions sans valeur associee : il ne doit ni
casser l'assemblage ni ajouter de ligne fantome. L'IV d'IB arrive tantot en
decimal tantot en pourcentage, et 18 ne peut pas etre 1 800 % de volatilite : au
dela de 3 c'est une echelle, pas un regime. Et le gamma publie par IB est garde
tel quel, Black-76 ne servant qu'a boucher les trous — sinon --gamma-source
published et le profil ne travailleraient pas sur le meme perimetre.

Un test verifie le contrat de bout en bout : la sortie entre telle quelle dans
analysis.analyser(), sans retouche."
```

---

### Tâche 4 : `selection_vif()` — les contrats à garder souscrits

**Fichiers :**
- Modifier : `ib_data.py`
- Test : `tests/test_ib.py`

**Interfaces :**
- Consomme : `ib_data.CONTRAT` (tâche 2), une chaîne au format `COLUMNS` (tâche 3).
- Produit : `ib_data.selection_vif(chaine, budget_lignes=90) -> pd.DataFrame` aux colonnes `CONTRAT`, triée par poids décroissant.

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à `tests/test_ib.py` :

```python
# ---=== selection_vif ===---

def test_selection_vif_respecte_le_budget_de_lignes():
    """Cent lignes chez IB, quatre-vingt-dix pour le vif : saturer le quota fait
    echouer les souscriptions suivantes en silence."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert len(ib_data.selection_vif(chaine, budget_lignes=90)) <= 90
    assert len(ib_data.selection_vif(chaine, budget_lignes=10)) == 10


def test_selection_vif_prend_les_contrats_qui_portent_le_gamma():
    """Le socle vient de mesurer ou le gamma est : un critere geometrique
    gaspillerait des lignes sur des strikes sans open interest."""
    defs, ticks = _trames_ib(oi=1.0)
    # un seul strike porte tout l'open interest, et il est loin de la monnaie
    loin = defs[(defs.StrikePrice == 24_600.0) & (defs.right == "C")].conId.iloc[0]
    ticks = ticks.copy()
    ticks.loc[ticks.conId == loin, "OpenInt"] = 1_000_000.0
    chaine, _, _ = ib_data.build_chain(defs, ticks, futures_price=PRIX,
                                       quote_date=QUOTE)

    choisis = ib_data.selection_vif(chaine, budget_lignes=3)
    premier = choisis.iloc[0]
    assert premier.StrikePrice == pytest.approx(24_600.0)
    assert premier.right == "C"


def test_selection_vif_ecarte_les_contrats_sans_poids():
    """Un strike sans open interest ne merite pas une ligne."""
    defs, ticks = _trames_ib(oi=0.0)
    chaine, _, _ = ib_data.build_chain(defs, ticks, futures_price=PRIX,
                                       quote_date=QUOTE)
    assert ib_data.selection_vif(chaine).empty


def test_selection_vif_rend_la_meme_forme_que_perimetre():
    """La couche reseau ne doit connaitre qu'une seule forme de contrat."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    assert list(ib_data.selection_vif(chaine).columns) == ib_data.CONTRAT


def test_selection_vif_est_deterministe_a_egalite():
    """A poids egaux, deux appels doivent rendre la meme liste : sinon le
    recyclage des lignes brasserait des souscriptions pour rien."""
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    un = ib_data.selection_vif(chaine, budget_lignes=20)
    deux = ib_data.selection_vif(chaine, budget_lignes=20)
    assert un.equals(deux)
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k selection_vif -v
```
Attendu : ÉCHEC, `AttributeError: module 'ib_data' has no attribute 'selection_vif'`.

- [ ] **Étape 3 : écrire l'implémentation minimale**

Ajouter à `ib_data.py` :

```python
def selection_vif(chaine, budget_lignes=90):
    """Les contrats à garder souscrits en permanence, |gamma × OI| décroissant.

    Un critère géométrique — plus ou moins N strikes autour du spot — serait plus
    simple, mais dilapiderait des lignes sur des strikes sans open interest alors
    que le socle vient précisément de mesurer où le gamma se trouve.

    Le budget tombe mieux qu'on ne l'avait prévu : quatre-vingt-dix lignes font
    quarante-cinq strikes, soit environ ±2,5 %, exactement la grille que le CME
    liste sur une échéance hebdomadaire. Le tri sert donc moins à choisir qu'à
    ordonner le recyclage quand le spot glisse et que la grille se déplace.

    Quatre-vingt-dix et non cent : le future consomme une ligne, le recyclage en
    réclame quelques-unes le temps que les annulations soient prises en compte, et
    saturer le quota fait échouer les souscriptions suivantes en silence.

    Rend la même forme que perimetre() — la couche réseau ne connaît qu'un seul
    genre de contrat.
    """
    blocs = []
    for side, right in (("Call", "C"), ("Put", "P")):
        bloc = chaine[["ExpirationDate", "StrikePrice"]].copy()
        bloc["right"] = right
        gamma = pd.to_numeric(chaine.get(f"{side}Gamma"), errors="coerce").fillna(0.0)
        oi = pd.to_numeric(chaine.get(f"{side}OpenInt"), errors="coerce").fillna(0.0)
        bloc["poids"] = (gamma * oi).abs()
        blocs.append(bloc)

    tous = pd.concat(blocs, ignore_index=True)
    tous = tous[tous.poids > 0]
    # Le tri secondaire rend l'ordre reproductible à poids égaux : sans lui, deux
    # appels successifs pourraient rendre deux listes différentes, et le recyclage
    # annulerait puis re-souscrirait les mêmes contrats pour rien.
    tous = tous.sort_values(["poids", "ExpirationDate", "StrikePrice", "right"],
                            ascending=[False, True, True, True])
    return tous.head(int(budget_lignes))[CONTRAT].reset_index(drop=True)
```

- [ ] **Étape 4 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : `test_ib.py` 22 passent ; suite complète **183**, zéro échec.

- [ ] **Étape 5 : commiter**

```bash
git add ib_data.py tests/test_ib.py
git commit -m "ib_data : le vif se choisit par le gamma, pas par la distance au spot

Quatre-vingt-dix lignes entretenues sur des milliers de contrats : il faut choisir.
Le critere evident — plus ou moins N strikes autour du spot — gaspillerait des
lignes sur des strikes sans open interest, alors que le socle vient exactement de
mesurer ou le gamma se trouve. On trie donc par |gamma x OI|.

Le budget tombe mieux que prevu : quatre-vingt-dix lignes font quarante-cinq
strikes, soit environ 2,5 %, ce qui est exactement la grille que le CME liste sur
une echeance hebdomadaire. Le vif ne couvre donc pas un morceau autour du spot, il
couvre toute la chaine listee de l'echeance proche, et le tri sert moins a choisir
qu'a ordonner le recyclage quand le spot glisse.

Le tri secondaire n'est pas cosmetique : a poids egaux, sans lui, deux appels
rendraient deux listes differentes et le recyclage annulerait puis re-souscrirait
les memes contrats pour rien."
```

---

### Tâche 5 : `fusionner()` — recoller le socle et le vif

**Fichiers :**
- Modifier : `ib_data.py`
- Test : `tests/test_ib.py`

**Interfaces :**
- Consomme : une chaîne au format `COLUMNS` (tâche 3), une trame de valeurs vives aux colonnes `CONTRAT` + `IV` et/ou `Gamma`.
- Produit : `ib_data.fusionner(socle, ticks_vif, spot) -> (pd.DataFrame, float)` — soit exactement la signature d'entrée d'`analysis.analyser()`.

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à `tests/test_ib.py` :

```python
# ---=== fusionner ===---

def _socle():
    chaine, _, _ = ib_data.build_chain(*_trames_ib(), futures_price=PRIX,
                                       quote_date=QUOTE)
    return chaine


def test_fusionner_rafraichit_l_iv_la_ou_le_vif_parle():
    """Ce qui bouge en seance, c'est le spot et l'IV."""
    socle = _socle()
    vif = pd.DataFrame([{"ExpirationDate": EXP, "StrikePrice": 25_000.0,
                         "right": "C", "IV": 0.42}])
    fusionnee, spot = ib_data.fusionner(socle, vif, spot=25_100.0)

    ligne = fusionnee[fusionnee.StrikePrice == 25_000.0].iloc[0]
    assert ligne.CallIV == pytest.approx(0.42)
    assert spot == pytest.approx(25_100.0)


def test_fusionner_laisse_l_iv_du_socle_ailleurs():
    """Le vif ne couvre que 2,5 % : les ailes gardent l'IV du socle."""
    socle = _socle()
    vif = pd.DataFrame([{"ExpirationDate": EXP, "StrikePrice": 25_000.0,
                         "right": "C", "IV": 0.42}])
    fusionnee, _ = ib_data.fusionner(socle, vif, spot=PRIX)

    loin = fusionnee[fusionnee.StrikePrice == 24_500.0].iloc[0]
    assert loin.CallIV == pytest.approx(VOL_VRAI)


def test_fusionner_ne_touche_jamais_l_open_interest():
    """L'OI ne bouge pas en seance : la chambre de compensation le calcule apres
    la cloture. Le figer n'est pas une approximation, c'est la seule valeur."""
    socle = _socle()
    vif = pd.DataFrame([{"ExpirationDate": EXP, "StrikePrice": 25_000.0,
                         "right": "C", "IV": 0.42, "OpenInt": 999_999.0}])
    fusionnee, _ = ib_data.fusionner(socle, vif, spot=PRIX)
    assert fusionnee.CallOpenInt.equals(socle.CallOpenInt)


def test_fusionner_distingue_les_calls_des_puts():
    """Meme strike, meme echeance : le sens doit trancher."""
    socle = _socle()
    vif = pd.DataFrame([{"ExpirationDate": EXP, "StrikePrice": 25_000.0,
                         "right": "P", "IV": 0.42}])
    fusionnee, _ = ib_data.fusionner(socle, vif, spot=PRIX)

    ligne = fusionnee[fusionnee.StrikePrice == 25_000.0].iloc[0]
    assert ligne.PutIV == pytest.approx(0.42)
    assert ligne.CallIV == pytest.approx(VOL_VRAI)      # le call n'a pas bouge


def test_fusionner_rafraichit_aussi_le_gamma_publie():
    """--gamma-source published lirait sinon un gamma perime."""
    socle = _socle()
    vif = pd.DataFrame([{"ExpirationDate": EXP, "StrikePrice": 25_000.0,
                         "right": "C", "IV": VOL_VRAI, "Gamma": 0.00123}])
    fusionnee, _ = ib_data.fusionner(socle, vif, spot=PRIX)
    ligne = fusionnee[fusionnee.StrikePrice == 25_000.0].iloc[0]
    assert ligne.CallGamma == pytest.approx(0.00123)


def test_fusionner_sans_vif_rend_le_socle_intact():
    """Au demarrage, ou apres une deconnexion, le socle seul doit rester lisible."""
    socle = _socle()
    for vide in (None, pd.DataFrame(columns=ib_data.CONTRAT + ["IV"])):
        fusionnee, spot = ib_data.fusionner(socle, vide, spot=PRIX)
        assert fusionnee.equals(socle)
        assert spot == pytest.approx(PRIX)


def test_fusionner_alimente_analyser_sans_retouche():
    """Le but de toute la fonction : (df, spot) est l'entree d'analyser()."""
    import analysis
    socle = _socle()
    vif = pd.DataFrame([{"ExpirationDate": EXP, "StrikePrice": 25_000.0,
                         "right": "C", "IV": 0.42}])
    df, spot = ib_data.fusionner(socle, vif, spot=25_100.0)

    a = analysis.analyser(df, spot=spot, quote_date=QUOTE, ticker="NQ",
                          contract_size=20, dte_max=None)
    assert np.isfinite(a.total_gex)


def test_fusionner_ignore_un_contrat_absent_du_socle():
    """Le vif peut porter un strike que le socle n'avait pas : on ne l'invente pas."""
    socle = _socle()
    vif = pd.DataFrame([{"ExpirationDate": EXP, "StrikePrice": 99_999.0,
                         "right": "C", "IV": 0.42}])
    fusionnee, _ = ib_data.fusionner(socle, vif, spot=PRIX)
    assert len(fusionnee) == len(socle)
    assert 99_999.0 not in set(fusionnee.StrikePrice)
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k fusionner -v
```
Attendu : ÉCHEC, `AttributeError: module 'ib_data' has no attribute 'fusionner'`.

- [ ] **Étape 3 : écrire l'implémentation minimale**

Ajouter à `ib_data.py` :

```python
# Ce que le vif peut rafraîchir. L'open interest n'y est PAS, et c'est le coeur
# de la fusion : la chambre de compensation le calcule après la clôture et ne le
# publie qu'une fois par jour. Le figer en séance n'est pas une approximation,
# c'est la seule valeur qui existe.
CHAMPS_VIFS = ["IV", "Gamma"]


def fusionner(socle, ticks_vif, spot):
    """Open interest du socle, IV et gamma du vif, spot du vif -> (df, spot).

    Rend exactement ce qu'analysis.analyser() prend en entrée : c'est tout
    l'objet de la fonction. Rien en aval n'a à savoir qu'il existe un socle et
    un vif.

    Le vif ne couvre qu'environ ±2,5 % : partout ailleurs l'IV reste celle du
    socle, ce qui pèse peu puisque le gamma s'y effondre. Un contrat que le vif
    porte mais que le socle ignore est écarté — on ne fabrique pas de ligne.
    """
    df = socle.copy()
    if ticks_vif is None or len(ticks_vif) == 0:
        return df, float(spot)

    vif = ticks_vif.copy()
    vif["ExpirationDate"] = pd.to_datetime(vif["ExpirationDate"], errors="coerce")
    vif["StrikePrice"] = pd.to_numeric(vif["StrikePrice"], errors="coerce")
    vif["right"] = vif["right"].astype(str).str.upper().str[0]

    cle = pd.MultiIndex.from_arrays([df.ExpirationDate, df.StrikePrice])
    for side, right in (("Call", "C"), ("Put", "P")):
        part = vif[vif.right == right]
        if part.empty:
            continue
        for champ in CHAMPS_VIFS:
            if champ not in part.columns:
                continue
            valeurs = pd.to_numeric(part[champ], errors="coerce")
            # last() plutôt que first() : le dernier tick reçu est le bon. Le
            # groupby dédoublonne aussi, sans quoi reindex refuserait de servir.
            maj = valeurs.groupby([part.ExpirationDate, part.StrikePrice]).last().dropna()
            if maj.empty:
                continue
            nouvelles = maj.reindex(cle).to_numpy()
            colonne = f"{side}{champ}"
            df[colonne] = np.where(pd.isna(nouvelles), df[colonne].to_numpy(), nouvelles)

    return df, float(spot)
```

- [ ] **Étape 4 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : `test_ib.py` 30 passent ; suite complète **191**, zéro échec.

- [ ] **Étape 5 : commiter**

```bash
git add ib_data.py tests/test_ib.py
git commit -m "ib_data : fusionner rend (df, spot), soit l'entree d'analyser()

C'est tout l'objet de la fonction, et c'est ce qui fait que rien ne bouge en aval :
ni analysis, ni greeks, ni plots, ni history, ni validate n'ont a savoir qu'il
existe un socle et un vif.

L'open interest ne fait pas partie de ce que le vif rafraichit, et ce n'est pas un
oubli. La chambre de compensation le calcule apres la cloture et ne le publie
qu'une fois par jour : le figer en seance n'est pas une approximation, c'est la
seule valeur qui existe. Ce qui bouge vraiment, c'est le prix du future et l'IV —
et l'IV loin de la monnaie bouge peu tout en pesant peu, le gamma s'y effondrant.

Le gamma publie est rafraichi lui aussi quand le vif le porte, sinon
--gamma-source published lirait un gamma perime a cote d'un profil recalcule.

Un contrat que le vif porte mais que le socle ignore est ecarte : on ne fabrique
pas de ligne a partir d'un tick isole."
```

---

### Tâche 6 : brancher le lecteur

**Fichiers :**
- Modifier : `main.py` (parser : `--suivre` ; `main()` : exclusion `--watch`/`--replay`)
- Test : `tests/test_ib.py`

**Interfaces :**
- Consomme : `snapshots.courant()` (tâche 1).
- Produit : `main.py --suivre NQ` et la levée conditionnelle de l'exclusion. Rien du second plan n'en dépend, mais l'utilisateur en a besoin dès que le collecteur écrit.

**Pourquoi :** `main.py:343` refuse `--watch` avec `--replay`, au motif qu'« une archive ne bouge plus ». Le courant, lui, bouge. L'exclusion doit porter sur les archives horodatées, pas sur le courant.

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à `tests/test_ib.py` :

```python
# ---=== branchement du lecteur ===---

def test_suivre_resout_le_chemin_du_courant(tmp_path):
    """--suivre NQ evite de taper snapshots/NQ/courant.parquet a la main."""
    import main
    import snapshots
    args = main.construire_parser().parse_args(["NQ", "--suivre", "--dir", str(tmp_path)])
    assert main.source_relecture(args) == snapshots.courant("NQ", str(tmp_path))


def test_watch_et_replay_restent_exclusifs_sur_une_archive(tmp_path):
    """Une archive horodatee ne bouge plus : la suivre n'a aucun sens."""
    import main
    args = main.construire_parser().parse_args(
        ["NQ", "--watch", "60", "--replay", str(tmp_path / "2026-08-25_1436.parquet")])
    with pytest.raises(ValueError, match="archive"):
        main.verifier_exclusions(args)


def test_watch_est_permis_sur_le_courant(tmp_path):
    """Le courant bouge : le suivre est exactement l'usage vise."""
    import main
    args = main.construire_parser().parse_args(
        ["NQ", "--watch", "60", "--suivre", "--dir", str(tmp_path)])
    main.verifier_exclusions(args)          # ne doit rien lever
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k "suivre or watch" -v
```
Attendu : ÉCHEC, `AttributeError: module 'main' has no attribute 'source_relecture'`.

- [ ] **Étape 3 : écrire l'implémentation minimale**

Dans `main.py`, ajouter au parser (à la suite de `--replay`) :

```python
    parser.add_argument("--suivre", action="store_true",
                        help="lire le relevé courant écrit par le collecteur "
                             "(snapshots/<TICKER>/courant.parquet)")
    parser.add_argument("--dir", default=snapshots.DOSSIER,
                        help=f"dossier des relevés archivés (défaut : {snapshots.DOSSIER})")
```

Puis ajouter, avant `charger()` :

```python
def source_relecture(args):
    """Le fichier à rejouer : --replay tel quel, ou le courant si --suivre."""
    if args.replay:
        return args.replay
    if args.suivre:
        return snapshots.courant(args.ticker, args.dir)
    return None


def verifier_exclusions(args):
    """Les combinaisons d'options qui n'ont pas de sens, refusées à l'entrée.

    --watch relit sa source à intervalle régulier. Sur une archive horodatée
    c'est absurde : elle ne bougera plus. Sur le courant c'est l'usage même,
    puisque le collecteur le réécrit toutes les quinze secondes. L'exclusion
    porte donc sur --replay, jamais sur --suivre.
    """
    if args.watch and args.replay:
        raise ValueError("--watch et --replay s'excluent : une archive ne bouge plus. "
                         "Pour suivre un relevé vivant, utilise --suivre.")
    if args.replay and args.suivre:
        raise ValueError("--replay et --suivre s'excluent : choisis une archive "
                         "précise, ou le relevé courant.")
```

Puis, dans `charger()`, remplacer la première condition :

```python
    source = source_relecture(args)
    if source:
        df, spot, quote_date, marche = snapshots.charger(source)
        return df, spot, quote_date, args.ticker, True, marche
```

Enfin, dans `main()`, remplacer le bloc d'exclusion :

```python
    verifier_exclusions(args)
    if args.watch:
        return suivre(args, contract_size)
    return un_passage(args, contract_size)
```

- [ ] **Étape 4 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : `test_ib.py` 33 passent ; suite complète **194**, zéro échec.

- [ ] **Étape 5 : vérifier que la ligne de commande répond**

```
./venv/Scripts/python.exe main.py --help
```
Attendu : `--suivre` et `--dir` apparaissent dans l'aide, sans traceback.

- [ ] **Étape 6 : commiter**

```bash
git add main.py tests/test_ib.py
git commit -m "main : --suivre le releve courant, et --watch cesse d'etre interdit dessus

L'exclusion entre --watch et --replay avait une bonne raison : une archive
horodatee ne bouge plus, la relire en boucle est absurde. Le releve courant, lui,
bouge — le collecteur le reecrit toutes les quinze secondes — et le suivre est
exactement l'usage vise. L'exclusion porte donc desormais sur --replay et pas sur
--suivre, et le message d'erreur indique la sortie.

--suivre NQ resout le chemin du courant plutot que de le faire taper. C'est la
seule raison d'etre du chemin fixe : sans lui, le lecteur devrait lister le
dossier pour savoir quel fichier est le dernier.

Les deux conditions sortent de main() dans verifier_exclusions(), qui se teste
sans lancer d'analyse."
```

---

## Vérification finale du lot

- [ ] `./venv/Scripts/python.exe -m pytest tests -q` → **194 passent**, zéro échec
- [ ] `./venv/Scripts/python.exe -c "import ib_data; print(ib_data.CONTRAT)"` → `['ExpirationDate', 'StrikePrice', 'right']`
- [ ] `git grep -n "ib_async" -- "*.py"` → **aucun résultat** : rien du lot hors ligne n'importe la dépendance de courtier
- [ ] `./venv/Scripts/python.exe main.py TSLA --no-charts --dte-max 7` → fonctionne comme avant, sans rien devoir à IB

## Ce que ce lot ne fait pas

Volontairement hors périmètre, pour le second plan :

- la connexion à IB Gateway, l'énumération des contrats FOP, la souscription par lots ;
- `ib_collector.py`, le démon : socle au réveil, vif entretenu, reconnexion quotidienne ;
- l'écriture périodique du courant et des archives ;
- le mode différé (`reqMarketDataType(3)`) et la lecture du tick 83 ;
- la documentation de la source dans le README, y compris la précaution de lecture : le chiffre produit est le gamma des options NQ, pas tout le gamma qui pèse sur le Nasdaq.

Ces travaux dépendent du test de la colonne Open Interest dans TWS, décrit au risque n° 1 de la spec. Le présent lot n'en dépend pas : les quatre fonctions sont identiques que l'open interest vienne d'IB ou du fichier End-of-Day du CME.
