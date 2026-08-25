# Collecteur IB pour NQ — couche réseau et démon : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**But :** connecter `ib_data.py` à IB, balayer le socle, entretenir le vif, et écrire le relevé courant en continu — de sorte que `python main.py NQ --suivre` affiche un GEX qui bouge.

**Architecture :** la couche réseau reste mince et sépare ce qui se teste de ce qui ne se teste pas. Deux fonctions pures nouvelles portent tout le raisonnement traduisible — le découpage en lots et la lecture d'un `Ticker` — et les fonctions de réseau ne font que boucler autour. `ib_collector.py` orchestre, avec ses décisions elles aussi isolées en fonctions pures.

**Pile :** Python 3.11+, pandas, numpy, `ib_async>=1.0` (extra `[ib]`, déjà déclaré).

**Spec :** `docs/superpowers/specs/2026-08-25-collecteur-ib-nq-design.md`

**Lot précédent :** `docs/superpowers/plans/2026-08-25-collecteur-ib-nq-hors-ligne.md` — les cinq fonctions pures d'assemblage, terminées, 204 tests.

## Contraintes générales

- **Aucun test automatisé ne touche au réseau.** Ce qui exige TWS est vérifié à la main, et le plan dit comment.
- **Lecture seule de bout en bout.** `IB.connect(..., readonly=True)`. Le collecteur ne passe jamais d'ordre et ne doit pas pouvoir en passer.
- **Rien du cœur n'importe `ib_async`.** L'import se fait dans la fonction qui s'en sert, jamais en tête de module — `python main.py TSLA` ne doit pas payer une dépendance de courtier.
- **Budget de lignes : 90 par défaut**, paramétrable. Cent est le plafond IB par défaut ; le future en consomme une et le recyclage quelques-unes.
- **Point de départ :** 204 tests passent (`./venv/Scripts/python.exe -m pytest tests -q`).

## Ce que le sondage du 25 août 2026 a établi

Ces faits sont mesurés, pas supposés. Le code en dépend directement.

| Fait | Valeur |
|---|---|
| `reqSecDefOptParams` | rend une entrée **par classe de cotation**, chacune avec **une** échéance et ses strikes |
| Classes sur NQ | 14 sur 24 jours ; 3 348 strikes ; **6 696 contrats** |
| `reqContractDetails(FuturesOption(exp))` | rend les contrats réels de cette échéance, `conId` compris |
| Open interest | `callOpenInterest` sur un call, `putOpenInterest` sur un put |
| Greeks | `ticker.modelGreeks` : `impliedVol`, `delta`, `gamma`, `vega`, `theta`, **`undPrice`** |
| Prix du future | servi par `undPrice` dans **chaque** tick d'option — mesuré à 29 207 |
| Carnet | `bid`, `ask`, `bidSize`, `askSize`, `close`, `high`, `low`, `last`, `volume` |
| **Prix absent** | IB code « pas de prix » par **`-1`**, et « pas de taille » par **`0`** |
| Sans abonnement CME | `reqMarketDataType(1)` ne rend **rien** ; `reqMarketDataType(3)` rend **tout** |

## Structure des fichiers

| Fichier | Responsabilité | Action |
|---|---|---|
| `ib_data.py` | + `lots()`, `ligne_ticker()` (pures) et la couche réseau | modifier |
| `ib_collector.py` | le démon : socle, vif, écriture, reconnexion | **créer** |
| `tests/test_ib.py` | les tests des deux nouvelles fonctions pures | modifier |
| `pyproject.toml` | `ib_collector` dans `py-modules` | modifier |
| `README.md` | documenter la source IB et le collecteur | modifier |

---

### Tâche 1 : `lots()` — découper le périmètre en paquets

**Fichiers :**
- Modifier : `ib_data.py`
- Test : `tests/test_ib.py`

**Interfaces :**
- Consomme : rien.
- Produit : `ib_data.lots(contrats, taille=90) -> list[pd.DataFrame]`.

**Pourquoi :** c'est la seule chose qui rende le balayage possible, et elle se teste sans réseau. Un lot trop gros fait échouer les souscriptions en silence quand le quota de cent lignes est saturé.

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à `tests/test_ib.py` :

```python
# ---=== lots ===---

def test_lots_ne_depasse_jamais_la_taille():
    """Saturer le quota de cent lignes fait echouer les souscriptions en silence."""
    contrats = _contrats_cotes()
    paquets = ib_data.lots(contrats, taille=90)
    assert all(len(p) <= 90 for p in paquets)


def test_lots_ne_perd_ni_ne_duplique_aucun_contrat():
    """Un contrat oublie est un trou dans la chaine, un contrat double est une
    ligne payee deux fois."""
    contrats = _contrats_cotes()
    paquets = ib_data.lots(contrats, taille=90)
    recolles = pd.concat(paquets, ignore_index=True)
    assert len(recolles) == len(contrats)
    assert sorted(recolles.conId) == sorted(contrats.conId)


def test_lots_compte_juste():
    """Le nombre de lots est ce qui fixe le temps de balayage."""
    contrats = _contrats_cotes()          # 2 x nombre de strikes
    n = len(contrats)
    assert len(ib_data.lots(contrats, taille=90)) == -(-n // 90)
    assert len(ib_data.lots(contrats, taille=n)) == 1
    assert len(ib_data.lots(contrats, taille=n + 1)) == 1


def test_lots_vide_rend_une_liste_vide():
    """Une echeance sans contrat dans la plage n'est pas une panne."""
    assert ib_data.lots(pd.DataFrame(columns=["conId", "StrikePrice"])) == []


def test_lots_refuse_une_taille_absurde():
    """Zero lot de zero contrat boucle indefiniment : echouer bruyamment."""
    with pytest.raises(ValueError):
        ib_data.lots(_contrats_cotes(), taille=0)
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k "lots" -v
```
Attendu : ÉCHEC, `AttributeError: module 'ib_data' has no attribute 'lots'`.

- [ ] **Étape 3 : écrire l'implémentation**

Ajouter à `ib_data.py`, après `selection_vif()` :

```python
# Cent lignes de données simultanées chez IB, par défaut. Le future en consomme
# une, et le recyclage quelques-unes le temps que les annulations soient prises
# en compte : on ne demande donc jamais le quota entier.
BUDGET_LIGNES = 90


def lots(contrats, taille=BUDGET_LIGNES):
    """Découpe le périmètre en paquets souscriptibles d'un coup.

    C'est le nombre de lots qui fixe le temps de balayage, pas le nombre de
    contrats : chaque lot coûte une attente de stabilisation, et cette attente ne
    dépend pas de sa taille. Six mille sept cents contrats font soixante-quinze
    lots, soit trois à cinq minutes.
    """
    taille = int(taille)
    if taille < 1:
        raise ValueError(f"taille de lot absurde : {taille} (attendu : au moins 1)")
    if contrats is None or len(contrats) == 0:
        return []
    return [contrats.iloc[i:i + taille].reset_index(drop=True)
            for i in range(0, len(contrats), taille)]
```

- [ ] **Étape 4 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k lots -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : 5 passent ; suite complète **209**.

- [ ] **Étape 5 : commiter**

```bash
git add ib_data.py tests/test_ib.py
git commit -m "ib_data : decouper le perimetre en lots souscriptibles

Cent lignes de donnees simultanees chez IB, quatre-vingt-dix demandees : le
future en consomme une, le recyclage quelques-unes le temps que les annulations
soient prises en compte, et saturer le quota fait echouer les souscriptions
suivantes en silence plutot que bruyamment.

C'est le nombre de LOTS qui fixe le temps de balayage, pas le nombre de contrats :
chaque lot coute une attente de stabilisation, et cette attente ne depend pas de
sa taille. Les 6 696 contrats mesures sur NQ font soixante-quinze lots.

Un lot vide n'est pas une panne, une taille de zero l'est : elle bouclerait
indefiniment."
```

---

### Tâche 2 : `ligne_ticker()` — lire un `Ticker` IB

**Fichiers :**
- Modifier : `ib_data.py`
- Test : `tests/test_ib.py`

**Interfaces :**
- Consomme : `ib_data.CHAMPS_TICK` (lot précédent).
- Produit : `ib_data.ligne_ticker(ticker) -> dict` — les clés sont `conId` plus `CHAMPS_TICK`, plus `UndPrice`.

**Pourquoi c'est le cœur du lot :** c'est la seule traduction entre le vocabulaire d'IB et celui du projet, et **elle se teste entièrement hors ligne** avec des objets factices. Tout le reste de la couche réseau n'est qu'une boucle autour d'elle.

Trois pièges que le sondage a révélés et que cette fonction doit absorber :
1. IB code « pas de prix » par **`-1`**, pas par `NaN`. Un `-1` qui passerait dans la chaîne donnerait des primes négatives.
2. L'open interest est dans `callOpenInterest` **ou** `putOpenInterest` selon le `right` du contrat.
3. `modelGreeks` peut être `None` — un contrat illiquide qui ne répond pas.

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à `tests/test_ib.py` :

```python
# ---=== ligne_ticker ===---

class _Grecs:
    """Imite ib_async.OptionComputation : seuls les champs lus comptent."""
    def __init__(self, impliedVol=None, delta=None, gamma=None, vega=None,
                 theta=None, undPrice=None):
        self.impliedVol, self.delta, self.gamma = impliedVol, delta, gamma
        self.vega, self.theta, self.undPrice = vega, theta, undPrice


class _Contrat:
    def __init__(self, conId, right, strike=29_050.0):
        self.conId, self.right, self.strike = conId, right, strike


class _Ticker:
    """Imite ib_async.Ticker avec les valeurs relevees par le sondage."""
    def __init__(self, contract, **champs):
        self.contract = contract
        defauts = dict(bid=float("nan"), ask=float("nan"), bidSize=float("nan"),
                       askSize=float("nan"), last=float("nan"),
                       lastSize=float("nan"), close=float("nan"),
                       volume=float("nan"), callOpenInterest=float("nan"),
                       putOpenInterest=float("nan"), modelGreeks=None)
        defauts.update(champs)
        for cle, valeur in defauts.items():
            setattr(self, cle, valeur)


def test_ligne_ticker_lit_un_call():
    """Valeurs relevees sur Q4BQ6 C29050 le 25 aout 2026."""
    t = _Ticker(_Contrat(909426450, "C"),
                bid=199.5, ask=203.5, bidSize=2.0, askSize=2.0,
                last=207.0, close=134.5, volume=21.0,
                callOpenInterest=15.0, putOpenInterest=0.0,
                modelGreeks=_Grecs(impliedVol=0.2056, delta=0.8589,
                                   gamma=0.0015764, vega=1.5863, theta=-10.236,
                                   undPrice=29_206.85))
    ligne = ib_data.ligne_ticker(t)
    assert ligne["conId"] == 909426450
    assert ligne["OpenInt"] == pytest.approx(15.0)      # le CALL prend callOpenInterest
    assert ligne["Bid"] == pytest.approx(199.5)
    assert ligne["Ask"] == pytest.approx(203.5)
    assert ligne["BidSize"] == pytest.approx(2.0)
    assert ligne["Vol"] == pytest.approx(21.0)
    assert ligne["LastSale"] == pytest.approx(207.0)
    assert ligne["IV"] == pytest.approx(0.2056)
    assert ligne["Gamma"] == pytest.approx(0.0015764)
    assert ligne["Delta"] == pytest.approx(0.8589)
    assert ligne["UndPrice"] == pytest.approx(29_206.85)


def test_ligne_ticker_lit_un_put_du_bon_cote():
    """Valeurs relevees sur Q4BQ6 P29050 : l'OI est dans putOpenInterest."""
    t = _Ticker(_Contrat(909426451, "P"),
                callOpenInterest=0.0, putOpenInterest=39.0)
    assert ib_data.ligne_ticker(t)["OpenInt"] == pytest.approx(39.0)


def test_ligne_ticker_traduit_le_moins_un_en_absence():
    """IB code 'pas de prix' par -1. Le laisser passer donnerait des primes
    negatives, et un implied_vol calcule sur du vide."""
    t = _Ticker(_Contrat(1, "C"), bid=-1.0, ask=-1.0, last=-1.0, close=-1.0)
    ligne = ib_data.ligne_ticker(t)
    for champ in ("Bid", "Ask", "LastSale", "Settle"):
        assert np.isnan(ligne[champ]), f"{champ} vaut {ligne[champ]}, attendu NaN"


def test_ligne_ticker_garde_une_taille_nulle():
    """Zero au bid est une information — aucune quantite affichee — pas une
    absence de donnee. Contrairement au -1 des prix."""
    t = _Ticker(_Contrat(1, "C"), bidSize=0.0, askSize=0.0)
    ligne = ib_data.ligne_ticker(t)
    assert ligne["BidSize"] == pytest.approx(0.0)
    assert ligne["AskSize"] == pytest.approx(0.0)


def test_ligne_ticker_sans_grecs_ne_plante_pas():
    """Un contrat illiquide ne repond parfois jamais : modelGreeks reste None."""
    ligne = ib_data.ligne_ticker(_Ticker(_Contrat(1, "C"), callOpenInterest=7.0))
    assert ligne["OpenInt"] == pytest.approx(7.0)
    for champ in ("IV", "Gamma", "Delta", "Vega", "Theta", "UndPrice"):
        assert np.isnan(ligne[champ])


def test_ligne_ticker_prend_close_comme_settle():
    """Le reglement de la veille : c'est ce dont infer_futures_price a besoin
    quand undPrice manque."""
    t = _Ticker(_Contrat(1, "C"), close=134.5)
    assert ib_data.ligne_ticker(t)["Settle"] == pytest.approx(134.5)


def test_ligne_ticker_rend_toutes_les_cles_attendues():
    """build_chain lit CHAMPS_TICK : une cle manquante ferait une colonne vide
    sans que rien ne le signale."""
    ligne = ib_data.ligne_ticker(_Ticker(_Contrat(1, "P")))
    for champ in ib_data.CHAMPS_TICK:
        assert champ in ligne, f"{champ} absent de la ligne"
    assert "conId" in ligne and "UndPrice" in ligne


def test_ligne_ticker_alimente_build_chain():
    """Le contrat de bout en bout : des Tickers doivent traverser build_chain."""
    tickers = []
    for i, (k, r) in enumerate([(29_000.0, "C"), (29_000.0, "P"),
                                (29_050.0, "C"), (29_050.0, "P")]):
        tickers.append(_Ticker(
            _Contrat(500_000 + i, r, strike=k),
            bid=10.0, ask=11.0, callOpenInterest=20.0, putOpenInterest=30.0,
            modelGreeks=_Grecs(impliedVol=0.20, delta=0.5, gamma=0.0015,
                               vega=1.5, theta=-10.0, undPrice=29_020.0)))
    ticks = pd.DataFrame([ib_data.ligne_ticker(t) for t in tickers])
    defs = pd.DataFrame([{"conId": t.contract.conId,
                          "StrikePrice": t.contract.strike,
                          "ExpirationDate": EXP,
                          "right": t.contract.right} for t in tickers])

    chaine, prix, _ = ib_data.build_chain(defs, ticks, futures_price=None,
                                          quote_date=QUOTE)
    assert len(chaine) == 2
    assert chaine.CallOpenInt.sum() == pytest.approx(40.0)
    assert chaine.PutOpenInt.sum() == pytest.approx(60.0)
    assert prix == pytest.approx(29_020.0)      # undPrice a servi de prix
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k ligne_ticker -v
```
Attendu : ÉCHEC, `AttributeError: module 'ib_data' has no attribute 'ligne_ticker'`.

- [ ] **Étape 3 : écrire l'implémentation**

Ajouter à `ib_data.py`, après `lots()` :

```python
# IB ne dit pas « pas de valeur » par NaN mais par -1 sur les prix. Laisser
# passer un -1 donnerait des primes negatives, un implied_vol calcule sur du
# vide, et un GEX faux sans que rien ne le signale. Les TAILLES, elles, valent
# legitimement zero : aucune quantite affichee est une information.
PRIX_ABSENT = -1.0


def _prix(valeur):
    """Prix IB -> flottant, avec -1 traduit en absence."""
    if valeur is None:
        return np.nan
    valeur = float(valeur)
    if math.isnan(valeur) or valeur == PRIX_ABSENT:
        return np.nan
    return valeur


def _taille(valeur):
    """Taille IB -> flottant. Zero est une valeur, pas une absence."""
    return np.nan if valeur is None else float(valeur)


def ligne_ticker(ticker):
    """Un Ticker d'ib_async -> une ligne au vocabulaire du projet.

    C'est la seule traduction entre les noms d'IB et ceux du format pivot, et
    c'est pour ça qu'elle est pure : tout le reste de la couche réseau n'est
    qu'une boucle autour d'elle, et se teste donc contre TWS plutôt que dans la
    suite.

    Trois pièges, tous relevés au sondage du 25 août 2026 :

      - l'open interest est dans `callOpenInterest` OU `putOpenInterest` selon le
        sens du contrat, jamais dans les deux ;
      - `modelGreeks` vaut None pour un contrat qui ne répond pas ;
      - `undPrice` voyage avec les grecs : le prix du future arrive dans chaque
        tick d'option, ce qui évite de le demander séparément.
    """
    contrat = ticker.contract
    droit = str(getattr(contrat, "right", "") or "").upper()[:1]

    oi = ticker.callOpenInterest if droit == "C" else ticker.putOpenInterest
    grecs = ticker.modelGreeks

    def grec(nom):
        valeur = getattr(grecs, nom, None) if grecs is not None else None
        return np.nan if valeur is None else float(valeur)

    return {
        "conId": getattr(contrat, "conId", None),
        "OpenInt": _taille(oi),
        "IV": grec("impliedVol"),
        "Gamma": grec("gamma"),
        "Delta": grec("delta"),
        "Vega": grec("vega"),
        "Theta": grec("theta"),
        # Le règlement de la veille, dont infer_futures_price a besoin si
        # undPrice venait à manquer.
        "Settle": _prix(ticker.close),
        "Bid": _prix(ticker.bid),
        "Ask": _prix(ticker.ask),
        "BidSize": _taille(ticker.bidSize),
        "AskSize": _taille(ticker.askSize),
        "Vol": _taille(ticker.volume),
        "LastSale": _prix(ticker.last),
        "UndPrice": grec("undPrice"),
    }
```

Ajouter `import math` en tête de `ib_data.py`, avant `import numpy as np`.

- [ ] **Étape 4 : faire lire `UndPrice` par `build_chain()`**

Sans ça, `ligne_ticker()` produirait un `UndPrice` que personne ne consomme, et
`build_chain(futures_price=None)` retomberait sur la parité call-put alors qu'IB
donne le prix directement. Dans `ib_data.py`, remplacer le bloc d'inférence par :

```python
    if futures_price is None:
        # IB sert le prix du sous-jacent dans modelGreeks.undPrice, donc dans
        # CHAQUE tick d'option. La parité call-put ne sert plus que de filet,
        # pour une source qui ne le donnerait pas — le CME et Databento.
        futures_price = _undprice(ticks)
    if futures_price is None:
        futures_price = infer_futures_price(chain)
    if futures_price is None:
        raise ValueError(
            "Prix du future indéterminable : ni undPrice, ni parité call-put "
            "exploitable. Passe-le avec --futures-price."
        )
    futures_price = float(futures_price)
```

et ajouter, juste avant `build_chain()` :

```python
def _undprice(ticks):
    """Le prix du sous-jacent tel qu'IB le sert avec les grecs, ou None.

    La médiane plutôt que la dernière valeur : les ticks d'un même balayage
    n'arrivent pas tous à la même seconde, et un contrat isolé peut porter un
    undPrice décalé sans que rien ne le signale.
    """
    if ticks is None or "UndPrice" not in getattr(ticks, "columns", []):
        return None
    valeurs = pd.to_numeric(ticks["UndPrice"], errors="coerce").dropna()
    valeurs = valeurs[valeurs > 0]
    return None if valeurs.empty else float(valeurs.median())
```

- [ ] **Étape 5 : lancer les tests pour vérifier qu'ils passent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k ligne_ticker -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : 8 passent, dont `test_ligne_ticker_alimente_build_chain` qui vérifie
que l'`undPrice` traverse et sert de prix ; suite complète **217**.

- [ ] **Étape 6 : commiter**

```bash
git add ib_data.py tests/test_ib.py
git commit -m "ib_data : lire un Ticker, et desamorcer les trois pieges d'IB

Seule traduction entre le vocabulaire d'IB et celui du projet, donc ecrite comme
fonction pure : tout le reste de la couche reseau n'est qu'une boucle autour
d'elle, et se verifie contre TWS plutot que dans la suite.

Trois pieges, tous releves au sondage. IB ne dit pas « pas de valeur » par NaN
mais par -1 sur les prix : le laisser passer donnerait des primes negatives et un
implied_vol calcule sur du vide, sans que rien ne le signale. Les TAILLES en
revanche valent legitimement zero — aucune quantite affichee est une information,
pas une absence, et les traiter comme les prix perdrait cette distinction.
L'open interest est dans callOpenInterest OU putOpenInterest selon le sens du
contrat. Et modelGreeks vaut None pour un contrat illiquide qui ne repond jamais.

undPrice voyage avec les grecs : le prix du future arrive dans chaque tick
d'option, donc on ne le demande pas separement. Un test verifie qu'il traverse
jusqu'a build_chain et y sert de prix."
```

---

### Tâche 3 : connexion et énumération

**Fichiers :**
- Modifier : `ib_data.py`
- Vérification : à la main contre TWS (aucun test automatisé)

**Interfaces :**
- Consomme : `echeances_utiles()`, `perimetre()` (lot précédent).
- Produit :
  - `ib_data.connecter(hote="127.0.0.1", port=7496, client_id=17, differe=None) -> IB`
  - `ib_data.front_month(ib, produit="NQ") -> Contract`
  - `ib_data.enumerer(ib, futur, dte_max=30, dte_min=0, quote_date=None) -> pd.DataFrame`
    aux colonnes `conId`, `StrikePrice`, `ExpirationDate`, `right`.

- [ ] **Étape 1 : écrire l'implémentation**

Ajouter à `ib_data.py`, à la suite de `ligne_ticker()` :

```python
# ---=== Accès réseau — non testable hors ligne ===---

HOTE_DEFAUT, PORT_DEFAUT = "127.0.0.1", 7496
CLIENT_ID_DEFAUT = 17

# 3 = différé. Sans abonnement CME temps réel, le type 1 ne rend RIEN — pas
# d'erreur, juste des champs vides — alors que le différé sert tout, open
# interest compris. Le mode retenu est annoncé à l'écran : le projet n'accepte
# pas qu'une donnée dégradée passe en silence.
MARCHE_TEMPS_REEL, MARCHE_DIFFERE = 1, 3


def connecter(hote=HOTE_DEFAUT, port=PORT_DEFAUT, client_id=CLIENT_ID_DEFAUT,
              differe=None):
    """Ouvre une session TWS en LECTURE SEULE.

    `differe=None` essaie le temps réel puis retombe sur le différé ; True ou
    False force. Le mode effectif est écrit sur la sortie standard.

    readonly=True n'est pas une précaution de style : le collecteur ne doit pas
    pouvoir passer d'ordre, même par un bug.
    """
    from ib_async import IB

    ib = IB()
    try:
        ib.connect(hote, port, clientId=client_id, timeout=15, readonly=True)
    except Exception as err:
        raise ValueError(
            f"Connexion à TWS impossible sur {hote}:{port} — {err}\n"
            "TWS ou IB Gateway tourne-t-il, et 'Enable ActiveX and Socket "
            "Clients' est-il coché dans Global Configuration > API > Settings ?"
        )

    mode = MARCHE_DIFFERE if differe else MARCHE_TEMPS_REEL
    ib.reqMarketDataType(mode)
    print(f"Connecté à {hote}:{port} — données "
          f"{'DIFFÉRÉES (~15 min)' if mode == MARCHE_DIFFERE else 'temps réel'}")
    return ib


def front_month(ib, produit="NQ", exchange="CME"):
    """Le future de première échéance : c'est lui que les options suivent."""
    from ib_async import Future

    details = ib.reqContractDetails(Future(produit, exchange=exchange))
    if not details:
        raise ValueError(f"Aucun future '{produit}' sur {exchange}.")
    proche = min(details,
                 key=lambda d: d.contract.lastTradeDateOrContractMonth)
    return proche.contract


def enumerer(ib, futur, dte_max=30, dte_min=0, quote_date=None, exchange="CME"):
    """Les contrats d'options RÉELLEMENT cotés dans l'horizon demandé.

    Deux appels de nature différente, et l'ordre compte.

    `reqSecDefOptParams` d'abord, uniquement pour les échéances : il rend une
    entrée PAR CLASSE DE COTATION — quatorze sur NQ — et chaque classe ne porte
    qu'une échéance avec ses propres strikes. Lire seulement la première en fait
    manquer treize.

    `reqContractDetails` ensuite, une fois par échéance retenue, le strike laissé
    indéfini : IB rend alors tous les contrats de cette échéance avec leurs
    conId. C'est la seule façon d'obtenir les couples existants. Croiser les
    strikes et les échéances de reqSecDefOptParams fabriquerait un produit
    cartésien — 11 872 contrats là où il n'en existe que 6 696, la majorité
    jamais cotée.
    """
    from ib_async import FuturesOption

    params = ib.reqSecDefOptParams(futur.symbol, exchange, "FUT", futur.conId)
    if not params:
        raise ValueError(
            f"reqSecDefOptParams ne rend rien pour {futur.symbol}. "
            "Sans liste d'échéances, il n'y a rien à énumérer."
        )
    toutes = sorted({e for classe in params for e in classe.expirations})
    retenues = echeances_utiles(toutes, quote_date, dte_max, dte_min)
    print(f"{len(params)} classes de cotation, {len(toutes)} échéances, "
          f"{len(retenues)} dans l'horizon")

    lignes = []
    for exp in retenues:
        jour = pd.Timestamp(exp).strftime("%Y%m%d")
        details = ib.reqContractDetails(
            FuturesOption(futur.symbol, lastTradeDateOrContractMonth=jour,
                          exchange=exchange))
        for d in details:
            c = d.contract
            if str(c.right).upper()[:1] not in ("C", "P"):
                continue
            lignes.append({"conId": c.conId, "StrikePrice": float(c.strike),
                           "ExpirationDate": pd.Timestamp(exp),
                           "right": str(c.right).upper()[:1]})
        print(f"  {jour} : {len(details)} contrats")

    if not lignes:
        raise ValueError("Aucun contrat d'option énuméré sur l'horizon demandé.")
    return pd.DataFrame(lignes)
```

- [ ] **Étape 2 : vérifier à la main contre TWS**

TWS doit tourner, API activée. Depuis la racine du projet :

```
./venv/Scripts/python.exe -c "
import ib_data
ib = ib_data.connecter(differe=True)
f = ib_data.front_month(ib)
print('future', f.localSymbol, f.conId, 'x' + str(f.multiplier))
c = ib_data.enumerer(ib, f, dte_max=7)
print(c.shape, c.ExpirationDate.nunique(), 'echeances')
print(c.head())
ib.disconnect()
"
```

Attendu : le future `NQU6`, une liste d'échéances, et une trame de plusieurs milliers de lignes aux quatre colonnes. **Si `reqSecDefOptParams` rend une liste vide, s'arrêter et le signaler** — c'est le risque n° 2 de la spec.

- [ ] **Étape 3 : vérifier que rien n'est cassé hors ligne**

```
./venv/Scripts/python.exe -m pytest tests -q
git grep -nE "^\s*(import|from)\s+ib_async" -- "*.py"
```
Attendu : **217** tests passent ; le `grep` ne rend **aucun** résultat — les imports d'`ib_async` sont tous dans le corps des fonctions.

- [ ] **Étape 4 : commiter**

```bash
git add ib_data.py
git commit -m "ib_data : se connecter, et enumerer les contrats qui existent

Deux appels de nature differente, et l'ordre compte. reqSecDefOptParams sert
UNIQUEMENT a lister les echeances : il rend une entree par classe de cotation —
quatorze sur NQ — chacune portant une seule echeance avec ses propres strikes,
si bien que ne lire que la premiere en fait manquer treize. reqContractDetails
ensuite, une fois par echeance, le strike laisse indefini : IB rend alors tous
les contrats de cette echeance avec leurs conId, et c'est la seule facon
d'obtenir les couples existants plutot qu'un produit cartesien de 11 872 contrats
dont la majorite n'a jamais ete cotee.

La connexion est en lecture seule, et ce n'est pas une precaution de style : le
collecteur ne doit pas pouvoir passer d'ordre, meme par un bug.

Le mode de donnees est annonce a l'ecran. Sans abonnement CME le temps reel ne
rend RIEN — pas d'erreur, juste des champs vides — quand le differe sert tout,
open interest compris. Une donnee degradee ne doit jamais passer en silence."
```

---

### Tâche 4 : `collecter()` — le balayage par lots

**Fichiers :**
- Modifier : `ib_data.py`
- Vérification : à la main contre TWS

**Interfaces :**
- Consomme : `lots()`, `ligne_ticker()` (tâches 1 et 2).
- Produit : `ib_data.collecter(ib, contrats, budget=BUDGET_LIGNES, attente=3.0, progres=True) -> pd.DataFrame` aux colonnes `conId` + `CHAMPS_TICK` + `UndPrice`.

- [ ] **Étape 1 : écrire l'implémentation**

Ajouter à `ib_data.py` :

```python
# Les generic ticks demandés à chaque souscription. 101 porte l'open interest des
# options, 588 celui des futures : rien ne documente lequel IB retient pour un
# FOP, et les demander ensemble ne coûte aucune ligne supplémentaire.
TICKS_GENERIQUES = "101,588"


def collecter(ib, contrats, budget=BUDGET_LIGNES, attente=3.0, progres=True):
    """Balaie le périmètre par lots et rend les valeurs reçues.

    Le mode snapshot n'accepte aucun generic tick — donc aucun open interest. Il
    faut souscrire en streaming, attendre, annuler, recommencer : c'est ce qui
    rend le socle long, et rien ne peut le contourner.

    `attente` est un délai de garde, pas une mesure : un contrat illiquide ne
    répond parfois jamais, et sans plafond le lot bloquerait indéfiniment.
    """
    paquets = lots(contrats, budget)
    recues = []
    for i, paquet in enumerate(paquets, 1):
        souscrits = []
        for ligne in paquet.itertuples():
            contrat = _contrat_ib(ligne)
            ib.reqMktData(contrat, TICKS_GENERIQUES, False, False)
            souscrits.append(contrat)

        ib.sleep(attente)
        for contrat in souscrits:
            recues.append(ligne_ticker(ib.ticker(contrat)))
            ib.cancelMktData(contrat)
        ib.sleep(0.2)   # laisser les annulations rendre leurs lignes

        if progres:
            print(f"\r  lot {i}/{len(paquets)} — {len(recues)} contrats lus",
                  end="", flush=True)
    if progres:
        print()
    return pd.DataFrame(recues)


def _contrat_ib(ligne):
    """Une ligne du périmètre -> un objet Contract, par conId.

    Le conId suffit et vaut mieux que le reste : il désigne exactement un
    contrat, là où symbole + échéance + strike + sens peut rester ambigu.
    """
    from ib_async import Contract

    return Contract(conId=int(ligne.conId), exchange="CME")
```

- [ ] **Étape 2 : vérifier à la main contre TWS**

```
./venv/Scripts/python.exe -c "
import ib_data, pandas as pd
ib = ib_data.connecter(differe=True)
f = ib_data.front_month(ib)
tous = ib_data.enumerer(ib, f, dte_max=2)
perim = ib_data.perimetre(tous, 29200.0, plage=0.01)
print(len(perim), 'contrats dans la plage,', len(ib_data.lots(perim)), 'lots')
ticks = ib_data.collecter(ib, perim.head(20), attente=4.0)
print(ticks[['conId','OpenInt','IV','Gamma','Bid','Ask','UndPrice']].to_string())
ib.disconnect()
"
```

Attendu : une vingtaine de lignes, avec `OpenInt`, `IV`, `Gamma` et `UndPrice` renseignés. **Si `Bid` ou `Ask` valent `-1`, `ligne_ticker()` a un défaut** — le `-1` doit devenir `NaN`.

- [ ] **Étape 3 : le socle complet, de bout en bout**

```
./venv/Scripts/python.exe -c "
import ib_data, analysis, snapshots
ib = ib_data.connecter(differe=True)
f = ib_data.front_month(ib)
tous = ib_data.enumerer(ib, f, dte_max=7)
perim = ib_data.perimetre(tous, 29200.0, plage=0.03)
ticks = ib_data.collecter(ib, perim)
defs = perim.rename(columns={})
chaine, prix, date_val = ib_data.build_chain(defs, ticks)
print('chaine', chaine.shape, 'future', prix)
a = analysis.analyser(chaine, spot=prix, quote_date=date_val, ticker='NQ',
                      contract_size=20, dte_max=None)
print('GEX total', a.total_gex, '| zero gamma', a.zero_gamma)
print(snapshots.sauver(chaine, 'NQ', prix, date_val))
ib.disconnect()
"
```

Attendu : un GEX fini et non nul, un zero gamma plausible près de 29 200, et un fichier écrit sous `snapshots/NQ/`.

- [ ] **Étape 4 : commiter**

```bash
git add ib_data.py
git commit -m "ib_data : balayer le socle par lots, souscrire puis annuler

Le mode snapshot de reqMktData n'accepte aucun generic tick, donc aucun open
interest : il faut souscrire en streaming, attendre la stabilisation, annuler,
recommencer. C'est ce qui rend le socle long, et rien ne peut le contourner.

L'attente est un delai de garde et non une mesure : un contrat illiquide ne
repond parfois jamais, et sans plafond le lot bloquerait indefiniment. On paie
donc ce delai a chaque lot, ce qui explique que le cout suive le nombre de lots
et non le nombre de contrats.

Les contrats sont designes par conId seul. C'est plus sur que symbole plus
echeance plus strike plus sens, combinaison qui peut rester ambigue quand
plusieurs classes de cotation coexistent sur la meme echeance — et il y en a
quatorze sur NQ.

101 et 588 sont demandes ensemble : rien ne documente lequel IB retient pour un
FOP, et les ticks generiques ne consomment aucune ligne supplementaire."
```

---

### Tâche 5 : `ib_collector.py` — les décisions, puis la boucle

**Fichiers :**
- Créer : `ib_collector.py`
- Test : `tests/test_ib.py`
- Modifier : `pyproject.toml` (`ib_collector` dans `py-modules`)

**Interfaces :**
- Consomme : tout `ib_data`, plus `snapshots.sauver()` et `snapshots.courant()`.
- Produit :
  - `ib_collector.faut_il_rebalayer(dernier_socle, maintenant) -> bool`
  - `ib_collector.faut_il_reselectionner(spot, bande) -> bool`
  - `ib_collector.bande_couverte(vif) -> (float, float)`
  - `ib_collector.collecter_en_boucle(...)` — la boucle, non testée automatiquement.

- [ ] **Étape 1 : écrire les tests des décisions**

Ajouter à `tests/test_ib.py` :

```python
# ---=== ib_collector : les decisions ===---

def test_rebalayage_une_fois_par_journee_de_compensation():
    """L'open interest ne bouge pas en seance : rebalayer plus souvent serait
    payer trois a cinq minutes pour relire le meme chiffre."""
    import ib_collector as ic
    socle = pd.Timestamp("2026-08-25 20:00")          # apres 18h Chicago
    assert not ic.faut_il_rebalayer(socle, pd.Timestamp("2026-08-25 23:00"))
    assert not ic.faut_il_rebalayer(socle, pd.Timestamp("2026-08-26 10:00"))
    assert ic.faut_il_rebalayer(socle, pd.Timestamp("2026-08-27 01:00"))


def test_rebalayage_si_aucun_socle():
    """Au demarrage il n'y a rien : il faut balayer."""
    import ib_collector as ic
    assert ic.faut_il_rebalayer(None, pd.Timestamp("2026-08-25 10:00"))


def test_bande_couverte_encadre_le_vif():
    """La bande dit jusqu'ou le vif voit, donc quand il cesse de voir."""
    import ib_collector as ic
    vif = pd.DataFrame({"StrikePrice": [28_900.0, 29_000.0, 29_100.0],
                        "ExpirationDate": [EXP] * 3, "right": ["C", "C", "P"]})
    bas, haut = ic.bande_couverte(vif)
    assert bas == pytest.approx(28_900.0)
    assert haut == pytest.approx(29_100.0)


def test_reselection_quand_le_spot_sort_de_la_bande():
    """Sans ca, les lignes entretenues finissent par regarder ailleurs que la ou
    ca se passe."""
    import ib_collector as ic
    bande = (28_900.0, 29_100.0)
    assert not ic.faut_il_reselectionner(29_000.0, bande)
    assert ic.faut_il_reselectionner(29_500.0, bande)
    assert ic.faut_il_reselectionner(28_000.0, bande)


def test_reselection_avec_une_marge_avant_le_bord():
    """Attendre la sortie franche serait attendre d'etre aveugle : on recycle
    quand le spot approche du bord."""
    import ib_collector as ic
    assert ic.faut_il_reselectionner(29_095.0, (28_900.0, 29_100.0), marge=0.10)


def test_reselection_sans_bande():
    """Aucun vif encore selectionne : il en faut un."""
    import ib_collector as ic
    assert ic.faut_il_reselectionner(29_000.0, None)
```

- [ ] **Étape 2 : lancer les tests pour vérifier qu'ils échouent**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k "rebalayage or bande or reselection" -v
```
Attendu : ÉCHEC, `ModuleNotFoundError: No module named 'ib_collector'`.

- [ ] **Étape 3 : écrire le module**

Créer `ib_collector.py` :

```python
"""Collecteur Interactive Brokers : socle au réveil, vif entretenu, relevé courant.

    python ib_collector.py NQ
    python ib_collector.py NQ --range 0.03 --dte-max 7 --differe

Le socle est un balayage complet du périmètre, refait une fois par journée de
compensation — l'open interest ne bouge pas en séance, le rebalayer plus souvent
serait payer trois à cinq minutes pour relire le même chiffre. Le vif entretient
quatre-vingt-dix souscriptions sur les contrats qui portent le gamma, et
rafraîchit l'IV et le prix du future en continu.

Le relevé courant est réécrit toutes les quinze secondes à chemin fixe, l'archive
horodatée toutes les quinze minutes. `python main.py NQ --suivre` lit le premier.

Le calcul n'est pas ici : ce module produit une chaîne, analysis.analyser() en
tire les chiffres. Les dupliquer créerait deux endroits où la même formule
pourrait diverger.
"""

import argparse
import os

import pandas as pd

import ib_data
import snapshots

# Le CME publie l'open interest préliminaire à 18 h heure de Chicago, soit 23 h
# UTC en heure d'été. Le socle est donc rebalayé au premier réveil qui suit, et
# une seule fois par journée de compensation.
HEURE_OI_UTC = 23

RAFRAICHIR_DEFAUT = 15        # secondes entre deux écritures du courant
ARCHIVER_DEFAUT = 15 * 60     # secondes entre deux archives horodatées
MARGE_BANDE = 0.15            # part de la bande au-delà de laquelle on recycle


def journee_compensation(instant):
    """La journée d'open interest à laquelle un instant appartient.

    Après 23 h UTC, on est déjà sur la publication du lendemain.
    """
    ts = pd.Timestamp(instant)
    return (ts + pd.Timedelta(hours=24 - HEURE_OI_UTC)).normalize()


def faut_il_rebalayer(dernier_socle, maintenant):
    """Le socle est-il périmé ?

    L'open interest est calculé par la chambre de compensation après la clôture
    et publié une fois par jour : rebalayer en cours de journée relirait le même
    chiffre pour trois à cinq minutes de souscriptions.
    """
    if dernier_socle is None:
        return True
    return journee_compensation(maintenant) > journee_compensation(dernier_socle)


def bande_couverte(vif):
    """Les strikes extrêmes que le vif surveille, ou None s'il est vide."""
    if vif is None or len(vif) == 0:
        return None
    strikes = pd.to_numeric(vif["StrikePrice"], errors="coerce").dropna()
    if strikes.empty:
        return None
    return float(strikes.min()), float(strikes.max())


def faut_il_reselectionner(spot, bande, marge=MARGE_BANDE):
    """Le spot approche-t-il du bord de ce que le vif surveille ?

    Attendre la sortie franche reviendrait à attendre d'être aveugle : quand le
    spot arrive près du bord, les contrats qui portent le gamma ne sont déjà plus
    ceux qu'on a souscrits.
    """
    if bande is None:
        return True
    bas, haut = bande
    largeur = haut - bas
    if largeur <= 0:
        return True
    garde = largeur * marge
    return not (bas + garde <= float(spot) <= haut - garde)


def collecter_en_boucle(ticker="NQ", plage=0.03, dte_max=7, dte_min=0,
                        budget=ib_data.BUDGET_LIGNES, rafraichir=RAFRAICHIR_DEFAUT,
                        archiver=ARCHIVER_DEFAUT, differe=True, dossier=snapshots.DOSSIER,
                        port=ib_data.PORT_DEFAUT, client_id=ib_data.CLIENT_ID_DEFAUT):
    """La boucle. Ctrl+C pour arrêter."""
    ib = ib_data.connecter(port=port, client_id=client_id, differe=differe)
    futur = ib_data.front_month(ib, ticker)
    print(f"Future {futur.localSymbol} (x{futur.multiplier})")

    socle = spot = vif_contrats = None
    date_socle = quote_date = dernier_archive = None

    try:
        while True:
            maintenant = pd.Timestamp.utcnow().tz_localize(None)

            # --- le socle, une fois par journée de compensation ---
            if faut_il_rebalayer(date_socle, maintenant):
                print(f"\n[{maintenant:%H:%M:%S}] Balayage du socle…")
                tous = ib_data.enumerer(ib, futur, dte_max, dte_min, maintenant)
                repere = spot if spot else _spot_amorce(ib, futur)
                perim = ib_data.perimetre(tous, repere, plage)
                print(f"  {len(perim)} contrats dans la plage, "
                      f"{len(ib_data.lots(perim, budget))} lots")
                ticks = ib_data.collecter(ib, perim, budget)
                socle, spot, quote_date = ib_data.build_chain(perim, ticks)
                date_socle = maintenant
                vif_contrats = None
                print(f"  socle : {len(socle)} strikes, future {spot:,.2f}")

            # --- le vif ---
            if faut_il_reselectionner(spot, bande_couverte(vif_contrats)):
                if vif_contrats is not None:
                    for ligne in vif_contrats.itertuples():
                        ib.cancelMktData(ib_data._contrat_ib(ligne))
                vif_contrats = ib_data.selection_vif(socle, budget)
                for ligne in vif_contrats.itertuples():
                    ib.reqMktData(ib_data._contrat_ib(ligne),
                                  ib_data.TICKS_GENERIQUES, False, False)
                bas, haut = bande_couverte(vif_contrats)
                print(f"[{maintenant:%H:%M:%S}] Vif : {len(vif_contrats)} contrats, "
                      f"bande {bas:,.0f}-{haut:,.0f}")

            ib.sleep(rafraichir)

            # --- fusion et écriture ---
            ticks_vif = pd.DataFrame(
                [ib_data.ligne_ticker(ib.ticker(ib_data._contrat_ib(l)))
                 for l in vif_contrats.itertuples()])
            ticks_vif = ticks_vif.join(
                vif_contrats[["ExpirationDate", "StrikePrice", "right"]])
            nouveau_spot = pd.to_numeric(ticks_vif.get("UndPrice"),
                                         errors="coerce").dropna()
            if not nouveau_spot.empty:
                spot = float(nouveau_spot.iloc[-1])

            fusionnee, spot = ib_data.fusionner(socle, ticks_vif, spot)
            # Le courant SEULEMENT : snapshots.sauver() horodate a la minute,
            # et l'appeler ici ecrirait quatre fichiers par minute. Les archives
            # ont leur propre cadence, plus bas.
            _ecrire(fusionnee, spot, quote_date,
                    snapshots.courant(ticker, dossier))
            print(f"\r[{maintenant:%H:%M:%S}] future {spot:,.2f} — "
                  f"{len(fusionnee)} strikes", end="", flush=True)

            if dernier_archive is None or (
                    maintenant - dernier_archive).total_seconds() >= archiver:
                snapshots.sauver(fusionnee, ticker, spot, maintenant, dossier)
                dernier_archive = maintenant
                print(f"\n[{maintenant:%H:%M:%S}] archive écrite")

    except KeyboardInterrupt:
        print("\nArrêt demandé.")
    finally:
        ib.disconnect()
        print("Déconnecté.")


def _ecrire(df, spot, quote_date, chemin):
    """Écrit le relevé courant au chemin fixe, format identique aux archives."""
    os.makedirs(os.path.dirname(chemin), exist_ok=True)
    out = df.copy()
    out[snapshots.META_SPOT] = float(spot)
    out[snapshots.META_DATE] = pd.Timestamp(quote_date)
    if chemin.endswith(".parquet"):
        out.to_parquet(chemin, index=False)
    else:
        out.to_csv(chemin, index=False, compression="gzip")
    return chemin


def _spot_amorce(ib, futur):
    """Prix du future au tout premier balayage, avant qu'aucun tick n'existe."""
    ticker = ib.reqMktData(futur, "", False, False)
    ib.sleep(3)
    for champ in ("last", "close", "bid"):
        valeur = ib_data._prix(getattr(ticker, champ, None))
        if valeur == valeur:          # non NaN
            ib.cancelMktData(futur)
            return valeur
    ib.cancelMktData(futur)
    raise ValueError("Prix du future indéterminable au démarrage.")


def main():
    p = argparse.ArgumentParser(description="Collecteur IB (socle + vif)")
    p.add_argument("ticker", nargs="?", default="NQ", help="produit CME (NQ, ES…)")
    p.add_argument("--range", type=float, default=0.03, dest="plage",
                   help="demi-plage de strikes autour du spot (0.03 = ±3%%)")
    p.add_argument("--dte-max", type=int, default=7, help="horizon en jours")
    p.add_argument("--dte-min", type=int, default=0)
    p.add_argument("--budget", type=int, default=ib_data.BUDGET_LIGNES,
                   help="lignes de données entretenues (100 max sans Quote Booster)")
    p.add_argument("--rafraichir", type=int, default=RAFRAICHIR_DEFAUT,
                   help="secondes entre deux écritures du relevé courant")
    p.add_argument("--archiver", type=int, default=ARCHIVER_DEFAUT,
                   help="secondes entre deux archives horodatées")
    p.add_argument("--temps-reel", action="store_true",
                   help="exiger le temps réel (défaut : différé, qui suffit)")
    p.add_argument("--dir", default=snapshots.DOSSIER)
    p.add_argument("--port", type=int, default=ib_data.PORT_DEFAUT)
    p.add_argument("--client-id", type=int, default=ib_data.CLIENT_ID_DEFAUT)
    args = p.parse_args()

    collecter_en_boucle(
        ticker=args.ticker, plage=args.plage, dte_max=args.dte_max,
        dte_min=args.dte_min, budget=args.budget, rafraichir=args.rafraichir,
        archiver=args.archiver, differe=not args.temps_reel, dossier=args.dir,
        port=args.port, client_id=args.client_id)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as err:
        raise SystemExit(f"Erreur : {err}")
```

- [ ] **Étape 4 : lancer les tests**

```
./venv/Scripts/python.exe -m pytest tests/test_ib.py -k "rebalayage or bande or reselection" -v
./venv/Scripts/python.exe -m pytest tests -q
```
Attendu : 6 passent ; suite complète **223**.

- [ ] **Étape 5 : déclarer le module**

Dans `pyproject.toml`, ajouter `"ib_collector"` après `"ib_data"` dans `py-modules`.

- [ ] **Étape 6 : vérifier à la main contre TWS**

```
./venv/Scripts/python.exe ib_collector.py NQ --range 0.02 --dte-max 2 --rafraichir 15
```

Laisser tourner deux ou trois minutes. Attendu : un balayage de socle, une
sélection de vif, puis une ligne qui se rafraîchit avec le prix du future. Dans
un autre terminal :

```
./venv/Scripts/python.exe main.py NQ --suivre --no-charts
```

Attendu : un GEX, un zero gamma, des murs — sur le relevé que le collecteur vient
d'écrire.

- [ ] **Étape 7 : commiter**

```bash
git add ib_collector.py tests/test_ib.py pyproject.toml
git commit -m "ib_collector : le socle une fois par jour, le vif en continu

Trois decisions sortent de la boucle pour se tester sans reseau, et ce sont les
seules qui portent un raisonnement. Le socle se rebalaie une fois par journee de
compensation et pas davantage : l'open interest est calcule apres la cloture et
publie une fois par jour, le relire en seance couterait trois a cinq minutes de
souscriptions pour le meme chiffre. Le vif se recycle quand le spot APPROCHE du
bord de sa bande, pas quand il en sort : attendre la sortie franche reviendrait a
attendre d'etre aveugle, les contrats qui portent le gamma ayant deja change.

Le module ne calcule rien. Il produit une chaine, analysis.analyser() en tire les
chiffres. Les dupliquer creerait deux endroits ou la meme formule pourrait
diverger, et c'est exactement ce que le depot a passe plusieurs commits a
defaire."
```

---

### Tâche 6 : documenter la source

**Fichiers :**
- Modifier : `README.md`
- Modifier : `AGENTS.md` (l'état du chantier)

**Interfaces :** aucune — de la documentation.

- [ ] **Étape 1 : écrire la section du README**

Ajouter après la section `### Databento : le 6E sans téléchargement manuel` :

```markdown
### Interactive Brokers : les options NQ en continu

```sh
python ib_collector.py NQ --range 0.03 --dte-max 7    # le collecteur, à laisser tourner
python main.py NQ --suivre                            # lire le relevé qu'il écrit
python main.py NQ --suivre --watch 30                 # et le relire toutes les 30 s
```

Il faut **TWS ou IB Gateway lancé**, avec l'API activée : `Global Configuration →
API → Settings`, cocher *Enable ActiveX and Socket Clients* et *Read-Only API*,
port 7496. Le collecteur se connecte en lecture seule et ne peut pas passer
d'ordre.

**Le socle et le vif.** IB ne sert pas une chaîne, il sert des contrats un par
un, avec cent lignes de données simultanées. Impossible d'y tenir six mille
contrats. Le collecteur balaie donc le périmètre entier une fois par journée de
compensation — c'est le *socle*, qui donne l'open interest — puis entretient
quatre-vingt-dix souscriptions sur les contrats qui portent le plus de gamma, le
*vif*, qui rafraîchit l'IV et le prix du future en continu.

Figer l'open interest en séance n'est pas une approximation : la chambre de
compensation le calcule après la clôture et ne le publie qu'une fois par jour.
C'est la seule valeur qui existe.

**Sans abonnement CME**, IB sert du différé d'un quart d'heure — et il sert
*tout*, open interest compris. Le collecteur s'en contente par défaut et
l'annonce à chaque démarrage. `--temps-reel` exige le direct, qui ne rend rien
sans l'abonnement.

**Ce que la mesure dit.** Le chiffre produit est le gamma des options **NQ**, pas
tout le gamma qui pèse sur le Nasdaq : la profondeur d'open interest de QQQ et
NDX n'existe pas côté futures. La mesure reste celle qu'emploient les
fournisseurs du domaine — Black-76, multiplicateur ×20 du CME — mais elle est
plus étroite qu'elle n'en a l'air.
```

- [ ] **Étape 2 : mettre le tableau Organisation à jour**

Remplacer la ligne `ib_data.py` par :

```markdown
| `ib_data.py` | source Interactive Brokers : périmètre, assemblage, sélection du vif |
| `ib_collector.py` | le collecteur : socle quotidien, vif entretenu, relevé courant |
```

- [ ] **Étape 3 : mettre `AGENTS.md` à jour**

Remplacer la section `## Chantier en cours` par un état à jour : le collecteur
tourne, le risque n° 1 est levé, restent le nettoyage de Databento et
`price_data.py` par IB.

- [ ] **Étape 4 : vérifier**

```
./venv/Scripts/python.exe -m pytest tests -q
grep -n "ib_collector\|--suivre\|Interactive Brokers" README.md | head
```
Attendu : 223 tests ; les trois motifs présents dans le README.

- [ ] **Étape 5 : commiter**

```bash
git add README.md AGENTS.md
git commit -m "README : documenter la source Interactive Brokers

Le socle et le vif expliques par ce qui les impose : IB ne sert pas une chaine
mais des contrats un par un, avec cent lignes simultanees, et six mille contrats
n'y tiennent pas. Figer l'open interest en seance n'est pas une approximation —
la chambre de compensation le calcule apres la cloture et ne le publie qu'une
fois par jour, c'est la seule valeur qui existe.

Le differe est le defaut assume : sans abonnement CME il sert tout, open interest
compris, et le collecteur l'annonce a chaque demarrage plutot que de laisser
croire au temps reel.

Et la precaution de lecture, qui vaut d'etre ecrite ou quelqu'un la lira : le
chiffre produit est le gamma des options NQ, pas tout le gamma qui pese sur le
Nasdaq."
```

---

## Vérification finale du lot

- [ ] `./venv/Scripts/python.exe -m pytest tests -q` → **223 passent**
- [ ] `git grep -nE "^\s*(import|from)\s+ib_async" -- "*.py"` → **aucun résultat**
- [ ] `./venv/Scripts/python.exe main.py TSLA --no-charts --dte-max 7` → inchangé, sans rien devoir à IB
- [ ] Le collecteur tourne dix minutes sans erreur, écrit le courant et au moins une archive
- [ ] `python main.py NQ --suivre` lit ce relevé et sort un GEX plausible

## Ce que ce lot ne fait pas

- **La reconnexion automatique après la coupure quotidienne.** La boucle s'arrête sur perte de connexion. À traiter une fois qu'on aura vu comment `ib_async` la signale en pratique — le deviner d'avance produirait du code non vérifiable.
- **Le nettoyage de `databento_data.py`**, décidé mais reporté à ce que le collecteur tourne.
- **`price_data.py` par IB** (`reqHistoricalData`), chantier distinct.
- **QQQ et NDX.** Seule la couche « quel contrat » changerait ; `build_chain()`, `selection_vif()` et `fusionner()` sont identiques.
