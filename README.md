<h1 align="center">Bienvenue pour l'Option Gamma Exposure et zero gamma level 👋</h1>
<p>
  <img alt="Version" src="https://img.shields.io/badge/version-1.0-blue.svg?cacheSeconds=2592000" />
  <a href="#" target="_blank">
    <img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-yellow.svg" />
  </a>
</p>

> Le projet a pour but de rendre option gamma exposure et zero gamma level accessible à tous.
> Les actions et indices US passent par l'API publique du CBOE (gratuite, sans clé) ; l'EUR/USD
> est scrapé sur barchart.com (options sur futures 6E). Le calcul s'appuie sur le script de
> https://perfiliev.com/author/perfiliev/.

### 🏠 [Homepage](https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level)

## Installation

```sh
git clone https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level.git
python -m venv venv
venv\Scripts\activate
pip install -r requirements.txt
```

## Utilisation

Actions et indices US, données récupérées automatiquement :

```sh
python main.py SPCX          # SpaceX
python main.py TSLA
python main.py _SPX          # les indices se préfixent d'un underscore
python main.py SPCX --range 0.35 --no-show
python main.py --csv spx_quotedata.csv   # export CSV manuel du site CBOE
```

Le script affiche le Total GEX, le **Zero Gamma Level**, le Call Wall et le Put Wall,
puis enregistre trois graphiques dans `charts/` :

| Fichier | Contenu |
|---|---|
| `<TICKER>_1_gamma_par_strike.png` | GEX net par strike |
| `<TICKER>_2_calls_vs_puts.png` | Décomposition gamma calls / puts |
| `<TICKER>_3_profil_zero_gamma.png` | Profil de gamma et zero gamma level |

### Horizon d'échéance (`--dte-max`)

Une chaîne CBOE porte plusieurs années d'échéances — jusqu'à 2031 sur le SPX. Prises
en bloc, les LEAPS écrasent l'analyse : leurs strikes ronds concentrent un OI énorme
mais purement spéculatif, qui ne produit aucun flux de hedging à court terme. Sur le
SPX du 10 août 2026, la chaîne complète donnait un GEX de **+70,8 Md$** (régime de
compression) alors que le 0–7 DTE — celui qui pilote réellement le hedging du jour —
donnait **−1,1 Md$**, soit le régime inverse.

`--dte-max` ne retient donc que les échéances proches, **défaut 30 jours calendaires** :

```sh
python main.py _SPX                    # 30 jours (défaut)
python main.py _SPX --dte-max 7        # semaine en cours
python main.py _SPX --dte-max all      # toute la chaîne
```

Le filtre s'applique avant tout calcul : murs, profil de gamma et zero gamma portent
toujours sur le même périmètre, rappelé dans l'en-tête et dans le titre des graphiques.
Les échéances déjà passées sont écartées dans la foulée.

`--wall-range` (défaut ±15 %) borne la recherche des murs autour du spot. Le Call Wall
est cherché **au-dessus** du spot et le Put Wall **en dessous** : sans cette contrainte
les deux tombent sur le strike ATM, où le gamma unitaire est maximal — au point de
battre des strikes dix fois plus chargés en OI — et le résultat ne fait que paraphraser
le spot.

Les murs pondérés par le gamma restent attirés vers la monnaie. Le script affiche donc
aussi les murs en **open interest brut** — la lecture « classique » — sur une bande plus
large, réglée par `--oi-wall-range` (défaut ±30 %) :

```
Call Wall  :     7,800.00 (gamma)       8,800.00 (open interest)
Put Wall   :     7,700.00 (gamma)       6,000.00 (open interest)
```

Sur une action les deux coïncident souvent. Sur un indice l'écart est net : ce relevé du
SPX donne 7 800 / 7 700 en gamma — soit le spot paraphrasé, à ±1 % — contre 8 800 / 6 000
en open interest. Les deux lectures répondent à des questions différentes : où le hedging
mord le plus, et où les positions sont réellement accumulées.

### Charm et vanna

Le gamma décrit la réaction à un mouvement de prix. Il ne dit rien des flux de couverture
déclenchés par **l'écoulement du temps** ni par **un choc de volatilité** — deux moteurs
majeurs dans les jours qui précèdent une échéance.

```
Charm      : -192.20 millions $ de delta / jour de bourse
Vanna      : +16.31 millions $ de delta / point de vol
```

**Charm** = variation du delta du book dealer par jour qui passe, à prix constant. Négatif
signifie que leur delta fond, donc qu'ils doivent **acheter** chaque jour pour rester
neutres : un flux de soutien mécanique, sans lien avec la direction du marché.

**Vanna** = variation du delta par point de volatilité implicite. Positif signifie que les
dealers vendent quand la vol monte — le canal par lequel un choc de vol se propage au spot.

Les deux sont calculés à `r = q = 0`, où ils sont identiques pour calls et puts (le `-1`
du delta put ne s'écoule pas), avec la même convention de signe que le GEX. `T` étant
exprimé en années **de bourse** (jours ouvrés / 262), le charm est ramené au jour de
bourse par ce même diviseur. Formules vérifiées par différence finie, et l'agrégat par
recalcul du delta dollar du book complet à un jour d'intervalle (écart 3 %, d'ordre deux).

Le quatrième graphique, `<TICKER>_4_charm_vanna.png`, les trace strike par strike.

### Historique

Chaque exécution ajoute une ligne à `history.csv` — sans quoi chaque analyse reste un
instantané et les séries n'existent nulle part.

```sh
python history.py              # dernier relevé de chaque ticker
python history.py SPCX         # la trajectoire d'un sous-jacent
python history.py SPCX --last 5
python main.py SPCX --no-history      # ne pas enregistrer
```

```
ticker                date       spot        GEX   zero gam   call w.    put w.    charm/j
SPCX      2026-08-03 16:00     114.85     -58.7M     137.51    160.00    100.00    -120.0M
SPCX      2026-08-06 16:00     110.48    -147.8M     124.02    115.00    110.00    -180.0M
SPCX      2026-08-10 16:00     134.10      +3.1M     132.02    150.00    150.00    -150.0M

variation sur 3 relevés :
  GEX              -58.7M ->        +3.1M       +61.8M   changement de signe
  le régime a changé de signe sur la période (gamma positif <-> négatif)
```

Le périmètre (`dte_max`) est enregistré avec chaque ligne : deux relevés du même jour sur
des horizons différents ne sont pas comparables, et rien d'autre ne les distinguerait.

### Validation : le modèle tient-il ?

```sh
python validate.py            # tous les tickers
python validate.py SPCX
```

Le modèle avance deux affirmations vérifiables, et `validate.py` les mesure sur
l'historique — sans source de prix externe, la colonne `spot` faisant office de série :

1. **les mouvements sont plus amples en gamma négatif** — amplitude médiane par jour,
   comparée entre les deux régimes, puis selon la position vis-à-vis du zero gamma ;
2. **le prix respecte les murs** — fréquence de franchissement du call wall et du put wall,
   dont les cas où le mur était à moins de 5 %.

En dessous de 20 intervalles, le script affiche les chiffres mais refuse d'en conclure
quoi que ce soit, et le dit. Il faut donc laisser l'historique s'accumuler — un relevé
par séance. Ce n'est pas un backtest de stratégie : on vérifie que la description du
terrain est exacte, pas qu'on peut en tirer de l'argent.

### Mesure du temps restant (`--time-convention`)

Le script de référence de Perfiliev compte le temps en **jours ouvrés / 262, avec un
plancher à 1 jour** pour les 0DTE. Ce plancher les surestime lourdement : un 0DTE à 10h
du matin, c'est 0,23 jour, pas 1. Le gamma variant en 1/√T, l'erreur est massive.

Mesuré sur le SPX, en comparant le gamma recalculé au gamma publié par le CBOE :

| convention | médiane | écart > 10 % |
|---|---|---|
| plancher 1 jour (Perfiliev) | 1,134 | 77 % |
| **heures restantes réelles** | **1,000** | **38 %** |

Par tranche, avec la convention correcte : 0j → 0,983, 1j → 1,012, 2j → 1,005, 5j → 0,986.
Le défaut est donc `heures` : temps réel jusqu'à 16h00 New York, rapporté à 365 jours.
`--time-convention bourse` restaure l'ancienne, pour reproduire le script de référence.

Sur le SPX cela déplace le zero gamma de 7 698 à 7 710 et le charm de −42,9 à −36,8 Md$.
Le charm est ramené au jour avec le diviseur de la convention active : `time_to_expiry()`
renvoie T et ce diviseur ensemble, précisément pour éviter de les désaccorder.

Indépendamment de la convention, le script signale les échéances très proches quand elles
pèsent plus de 20 % du GEX **ou du charm** — ce dernier bien plus exposé, variant en 1/T :

```
Attention : les échéances à 0-1 jour portent 18% du GEX, 73% du charm.
```

`--dte-min 2` les exclut ; comparer les deux lectures avant de conclure.

> Les greeks du CBOE, eux, **ne sont pas périmés** : mesuré en séance, le delta bouge sur
> 85 % des contrats et l'IV sur 96 % en trois minutes. Seul l'open interest est quotidien,
> et aucun fournisseur ne le publie en intraday.

### Tests

```sh
python -m pytest tests -q        # 96 tests, aucun accès réseau
```

Les greeks ne sont pas comparés à des valeurs codées en dur — celles-ci viendraient de la
même formule que le code et ne prouveraient rien. Charm et vanna sont recoupés par
**différences finies** sur le delta, le gamma Black-Scholes contre Black-76, et les parsers
contre des jeux construits depuis des paramètres connus (on vérifie qu'on retrouve le prix
du future, l'IV et le gamma injectés).

### Options sur futures (EUR/USD via le 6E, ES, ...)

Le CME interdit l'accès automatisé à son site (Data Terms of Use) : il n'y a donc pas
de scraper ici. On part d'un fichier téléchargé à la main depuis l'
[Option Settlement Tool](https://www.cmegroup.com/tools-information/quikstrike/option-settlement.html),
et `cme_data.py` le convertit au format du pipeline.

```sh
python main.py 6E --cme reglement_6E.csv --expiry 2026-09-04 --range 0.05
python main.py ES --cme reglement_ES.csv --contract-size 50
python -c "import cme_data; cme_data.inspect('reglement_6E.csv')"   # si le parsing échoue
```

Le CME ne publiant pas le gamma, il est calculé en **Black-76** à partir de la volatilité
implicite du fichier — et si elle est absente, elle est inversée depuis le prix de règlement.
Le prix du future est déduit par parité call-put s'il n'est pas fourni (`--futures-price`).
La détection des colonnes est tolérante (formats large et long, alias, casse libre).

À `r = q = 0`, le gamma Black-Scholes est identiquement égal au gamma Black-76 : le même
code sert donc aux actions et aux options sur futures, seul le multiplicateur change
(100 par défaut, 125 000 pour le 6E).

### Databento : le 6E sans téléchargement manuel

[Databento](https://databento.com) redistribue légalement les données CME Globex,
donc la chaîne complète est récupérable par API — sans scraping.

```sh
pip install databento
export DATABENTO_API_KEY=db-xxxxxxxx      # $env:DATABENTO_API_KEY="db-..." sous PowerShell

python main.py 6E --databento             # dernière séance close
python main.py 6E --databento --date 2026-08-03
python main.py ES --databento --contract-size 50
```

Deux requêtes par appel : le schéma `definition` fournit strike, échéance et
call/put, le schéma `statistics` fournit l'open interest (`stat_type` 9), le prix
de règlement et, quand le CME le publie, la volatilité implicite. Le prix du
future vient du règlement de première échéance, à défaut de la parité call-put.

Chaque requête est facturée au volume de données : une séance d'options 6E reste
modeste, mais évite les boucles sur de longues périodes.

### Barchart (options sur futures, gratuit mais aléatoire)

```sh
python Scrap-data.py E6U26 --expiry aug-26 --out barchart_6E.csv
python main.py 6E --cme barchart_6E.csv --expiry 2026-08-28
```

Le script fusionne deux vues — `volatility-greeks` (IV, gamma) et `options`
(volume, **open interest**, indispensable au GEX) — sur (strike, type), et écrit
un CSV que `cme_data.py` relit tel quel.

Barchart rend ses tableaux dans un shadow DOM (`<bc-data-grid>`) : le texte n'est
pas accessible par `.text`, il faut descendre dans le `shadowRoot`. Surtout, le
site **sert des cellules vides aux navigateurs automatisés selon l'adresse IP** :
les en-têtes chargent, les valeurs non. Le script échoue alors avec un message
explicite plutôt que d'écrire un fichier vide. Essayer `--visible` le cas échéant.

### Suivi du flux (qui achète, qui vend)

Le GEX dit *où* sont les positions, pas *qui* les a initiées. Pour ça il faut
comparer chaque transaction à la fourchette bid/ask du moment : au-dessus du mid
l'acheteur était à l'initiative, en dessous c'est le vendeur.

`flow_tracker.py` approche ça gratuitement en échantillonnant le CBOE : entre
deux relevés, un contrat dont le volume a augmenté **et** dont l'horodatage du
dernier trade a avancé fournit un trade neuf, situable dans sa fourchette.

```sh
python flow_tracker.py ORCL --interval 300 --duration 6h
python flow_tracker.py SPCX --interval 180 --out flux_spcx.csv
```

À lancer **pendant la séance** (9h30–16h ET), en comptant 15 minutes de plus :
le flux CBOE est différé d'autant, et hors séance volume et derniers trades
restent figés sur la clôture précédente. Le script prévient si le marché est
fermé. L'horodatage du payload CBOE est en UTC, celui des trades en heure de
New York — la classification compare les horodatages entre relevés plutôt qu'à
l'heure courante, ce qui rend le décalage sans effet.

Ses limites, à garder en tête : on ne voit que le **dernier** trade de chaque
fenêtre, dont le côté est appliqué à tout le volume de la fenêtre. Le signal
n'a de sens qu'agrégé sur de nombreux contrats. Pour de vrais prints il faut le
tape OPRA — Tradier (gratuit avec un compte), Polygon ou Databento.

#### Mesurer le sens au lieu de le supposer

Tout le reste du projet postule que les dealers sont longs calls et shorts puts.
C'est l'hypothèse la plus fragile de la méthode. Le flux collecté permet de la
remplacer par une mesure — la position dealer est le miroir du flux client agressif :

```
position_dealer[strike] = ventes_clients - achats_clients
```

```sh
python flow_tracker.py ORCL --signed flux_orcl.csv
```

```
GEX signé par le flux    :   -4.76 M$   (inventaire pris aujourd'hui)
GEX signé par convention : +470.78 M$   (structure accumulée, mêmes strikes)
-> SIGNES OPPOSÉS

contrats nets pris par les dealers : calls -6,598  puts +2,603
```

Ici les clients ont acheté des calls et vendu des puts, donc les dealers sont **shorts
calls** — l'inverse de ce que postule la convention. Les trades au milieu de la
fourchette sont écartés, pas devinés.

Deux réserves qui interdisent de substituer l'un à l'autre :

- cela mesure la **variation d'inventaire de la séance**, partant de zéro à l'ouverture,
  pas le book existant. Un strike non traité pèse zéro ici alors qu'il peut porter un
  open interest massif. Les ordres de grandeur ne sont donc pas comparables ;
- la classification reste grossière (voir plus haut), donc c'est une indication de sens,
  pas une mesure fine.

Les deux lectures sont complémentaires : la convention décrit la structure accumulée,
le flux décrit ce que les dealers ont pris aujourd'hui.

### Sources de données

| Module | Source | Accès |
|---|---|---|
| `cboe_data.py` | `cdn.cboe.com/api/global/delayed_quotes/options/{TICKER}.json` | libre, sans clé, différé |
| `cboe_data.load_from_csv()` | export CSV du site CBOE | téléchargement manuel |
| `cme_data.load_settlement()` | export du CME Option Settlement Tool | téléchargement manuel |
| `databento_data.fetch_chain()` | Databento GLBX.MDP3 | clé API, facturé à l'usage |

Le JSON CBOE fournit strike, expiration, OI, IV et gamma. `to_cboe_csv()` permet de
réécrire une chaîne au format CSV historique pour d'autres outils.

## Auteur

👤 **Taz**

* Site web: https://github.com/TazTheworld
* Github: [@TazTheWorld](https://github.com/TazTheWorld)

## 🤝 Contribuer

Les contributions, les problèmes et les demandes de fonctionnalités sont les bienvenus !<br />N'hésitez pas à consulter [issues page](https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level/issues). 

## Montrez votre soutien

Donnez un ⭐️ si ce projet vous a aidé !

***
_This README was generated with ❤️ by [readme-md-generator](https://github.com/kefranabg/readme-md-generator)_
