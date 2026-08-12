<h1 align="center">Bienvenue pour l'Option Gamma Exposure et zero gamma level 👋</h1>
<p>
  <img alt="Version" src="https://img.shields.io/badge/version-1.1-blue.svg?cacheSeconds=2592000" />
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

`requirements.txt` ne contient que le nécessaire pour les actions et indices US :
numpy, pandas, scipy, matplotlib, requests. Les autres sources sont des extras, à
n'installer que si on s'en sert :

```sh
pip install -e ".[databento]"    # chaînes CME par API
pip install -e ".[cme]"          # exports CME au format Excel
pip install -e ".[barchart]"     # scraping Barchart (selenium)
pip install -e ".[snapshots]"    # archivage en parquet plutôt qu'en csv.gz
pip install -e ".[dev]"          # tests
```

## Interface web locale

```sh
python serve.py            # http://127.0.0.1:8000, s'ouvre tout seul
python serve.py --port 8080 --no-browser
```

Une page unique pour lire un relevé : le régime en chiffre d'appel, les six niveaux
en tuiles, puis le profil de gamma, l'exposition par strike, la décomposition
calls / puts, le charm et la vanna. Les réglages — sous-jacent, horizon, source de
gamma, convention de temps, plage — sont en une seule rangée en haut, et
rejouent tout le relevé d'un coup.

Le serveur n'ajoute **aucune dépendance** : `http.server` de la bibliothèque
standard, du HTML et du JavaScript sans framework, des graphiques en SVG écrits à
la main. Il ne fait que servir `web/` et exposer `analysis.analyser()` en JSON,
donc la page ne peut afficher aucun chiffre que les tests ne couvrent pas. La
chaîne brute est gardée trois minutes en mémoire par sous-jacent : changer
d'horizon ou de source recalcule en local, sans retélécharger. Les relevés
archivés apparaissent dans un sélecteur, pour rejouer une séance passée.

Trois cartes s'ajoutent quand il y a de quoi les remplir, et disparaissent sinon
plutôt que d'afficher du vide : **la dérive** trace le prix et le zero gamma côte à
côte au fil des relevés enregistrés — c'est leur écart qui décrit le régime, et le
voir se refermer vaut mieux qu'une photo ; **le signe du flux** confronte la
convention au flux réellement observé dès qu'un `flux_<ticker>.csv` existe ; et le
sélecteur **Rafraîchir** rejoue le relevé à intervalle régulier, sans descendre sous
la minute puisque le flux CBOE est différé d'un quart d'heure.

Sur le fond clair, les couleurs de données sortent d'une palette validée
(bande de clarté, plancher de chroma, séparation en vision daltonienne,
contraste sur la surface). Le bleu et le rouge n'y encodent qu'une polarité —
la couverture amortit ou amplifie — jamais une identité. Les repères (spot,
zero gamma, murs) ne prennent aucune couleur de série : ce sont des pastilles
en encre, parce qu'un trait fin teinté ne porte pas le contraste. Chaque
graphique a une légende, une infobulle au survol et **une vue tableau** listant
exactement les mêmes strikes que le tracé : aucune valeur n'est accessible par
la seule couleur.

## Organisation

| Module | Rôle |
|---|---|
| `main.py` | interface en ligne de commande, rien d'autre |
| `serve.py` + `web/` | interface web locale |
| `greeks.py` | gamma, charm, vanna Black-Scholes, vectorisés — le seul endroit où une formule est écrite |
| `analysis.py` | filtre d'échéance, expositions, murs, profil, zero gamma |
| `plots.py` | les quatre graphiques |
| `snapshots.py` | archivage des chaînes brutes, pour rejouer une séance |
| `history.py` | historique des relevés |
| `validate.py` | le modèle tient-il ? |
| `cboe_data.py`, `cme_data.py`, `databento_data.py`, `barchart_data.py` | sources |
| `flow_tracker.py` | suivi du flux et signe réel de la position dealer |

Tout ce qui produit un chiffre est dans `analysis.py` et `greeks.py`, donc appelable
sans réseau et couvert par les tests. C'était auparavant enfermé dans `main()`.

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
puis enregistre quatre graphiques dans `charts/` :

| Fichier | Contenu |
|---|---|
| `<TICKER>_1_gamma_par_strike.png` | GEX net par strike |
| `<TICKER>_2_calls_vs_puts.png` | Décomposition gamma calls / puts |
| `<TICKER>_3_profil_zero_gamma.png` | Profil de gamma et zero gamma level |
| `<TICKER>_4_charm_vanna.png` | Charm et vanna par strike |

### D'où vient le gamma (`--gamma-source`)

Le CBOE publie un gamma par contrat, et on peut aussi le recalculer en Black-Scholes
depuis la volatilité implicite. Le script **mélangeait les deux sans le dire** : le
Total GEX prenait le gamma publié pendant que le profil — donc le Zero Gamma affiché
juste en dessous — recalculait depuis l'IV. Deux estimateurs pour deux chiffres
présentés comme cohérents.

Une seule source alimente désormais tout le pipeline, rappelée dans l'en-tête comme
dans le titre des graphiques :

```sh
python main.py _SPX                             # --gamma-source iv (défaut)
python main.py _SPX --gamma-source published    # le gamma tel que diffusé
```

Le défaut est `iv` parce que c'est le seul choix cohérent de bout en bout : à un
niveau de spot hypothétique, aucun gamma publié n'existe, donc le profil ne peut
être que recalculé. Avec `published`, le Total GEX et le Zero Gamma viennent
forcément d'estimateurs différents — le script le dit alors explicitement, avec
l'écart mesuré.

Une fois la mesure du temps corrigée (voir *Mesure du temps restant* plus bas), les
deux sources se rejoignent sur les horizons courants. Relevé du SPX du 12 août 2026 :

| horizon | gamma publié | recalculé (`iv`) | écart |
|---|---|---|---|
| ≤ 1 j | +5,06 Md | +5,64 Md | +11,4 % |
| ≤ 7 j | +15,37 Md | +15,62 Md | +1,7 % |
| ≤ 30 j (défaut) | +39,89 Md | +39,46 Md | −1,1 % |
| toute la chaîne | +87,92 Md | +76,96 Md | −12,5 % |

L'écart subsiste là où l'on s'y attend : sur les 0-1 DTE, où le gamma explose et
dépend du spot à la minute que des données différées ne donnent pas ; et sur les
LEAPS, où l'hypothèse `r = q = 0` cesse d'être neutre. Au-delà de 5 %, le script
le signale.

### Le GEX rapporté à ce qui s'échange

Un montant de couverture nu ne dit rien. Le payload CBOE porte déjà l'OHLCV et
l'iv30 du sous-jacent — le projet les téléchargeait sans les lire. Ils sont
désormais captés, archivés, et le rapport qui rend le GEX lisible est affiché :

```
Total GEX  : 125.55 millions $ / mouvement de 1%
GEX/volume : 0.9% du volume du jour (14.08 milliards $ échangés)
IV 30j     : 69.5%
```

Sous quelques pour cent, la couverture des dealers est un frottement, pas le
moteur de la séance. Au-delà du tiers, elle devient un acteur majeur du carnet.
Le même montant sur un titre cent fois moins liquide ne raconte pas la même
histoire — et sans cette division, rien ne le signale.

Ces colonnes entrent dans `history.csv` et dans les archives, ce qui débloque la
validation ci-dessous.

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

### Zero gamma : quel croisement ?

Le profil peut repasser par zéro plusieurs fois, dès que les ailes sont bruyantes. Le
script retenait le **premier** croisement de la fenêtre, donc le plus bas : sur un profil
croisant en 91,5 et 104,5 avec un spot à 100, il annonçait 91,5, soit 8,5 % sous le spot,
alors que la bascule de régime se joue juste au-dessus.

C'est maintenant le croisement **le plus proche du spot** qui est retenu — celui qui
délimite le régime dans lequel le marché se trouve effectivement. Les autres sont
signalés dans la sortie et tracés en pointillés sur le troisième graphique :

```
Attention : le profil croise zéro 2 fois (également en 7,412.30). Le niveau retenu est
le plus proche du spot ; le régime n'est pas une simple bascule au-dessus / en dessous.
```

### Ce que devient la volatilité quand le spot bouge (`--vol-regime`)

Le profil de gamma évalue l'exposition à des niveaux de spot hypothétiques. Reste
à décider ce que devient l'IV en chemin, et les deux réponses encadrent la réalité :

```sh
python main.py _SPX                                  # sticky-strike (défaut)
python main.py _SPX --vol-regime sticky-moneyness
```

- **sticky-strike** : chaque contrat garde son IV. Le prix glisse le long du skew
  existant, ce qui fait monter mécaniquement la vol à la monnaie quand le spot
  baisse. C'est l'hypothèse de la littérature.
- **sticky-moneyness** : le smile est figé en monnaie et se translate avec le spot.
  La vol à la monnaie reste constante ; un strike donné voit la sienne varier de
  `pente × ln(S₀/S′)`, la pente étant ajustée sur les strikes proches de la monnaie.

Le décalage vaut **exactement zéro au spot courant** et croît avec la distance.
Mesuré sur le SPX du 12 août :

| | zero gamma | aile gauche | aile droite |
|---|---|---|---|
| ≤ 7 j | +0,01 % du spot | −51 % | +87 % |
| ≤ 30 j | +0,02 % du spot | −35 % | +45 % |

Autrement dit : ce réglage remodèle les ailes du profil mais ne déplace quasiment
pas un zero gamma situé près du spot, ce qui est le cas courant. C'est un test de
robustesse du profil dans les ailes, pas une correction du niveau central — et le
Total GEX, mesuré au spot, n'en dépend pas du tout.

### Suivre une séance (`--watch`)

```sh
python main.py SPCX --watch 5m --watch-duration 6h --no-charts
```

L'open interest ne bouge qu'une fois par jour : en séance, seuls le spot et l'IV
changent. Le zero gamma ne se déplace donc pas beaucoup, mais la **distance** du
prix à ce niveau, elle, se referme ou s'ouvre — et c'est elle qui décide du régime.

```
  depuis le relevé précédent : spot +0.18, zero gamma +1.06, distance au zero gamma +9.72%
```

Un passage n'écrit dans l'historique que si le flux a réellement avancé : les données
CBOE étant différées d'un quart d'heure, deux passages rapprochés renvoient le même
relevé, et le réenregistrer n'ajouterait qu'une ligne identique.

### Rejouer une séance (`--replay`)

Chaque exécution archive la chaîne **brute** dans `snapshots/<TICKER>/<date>.parquet`
(quelques centaines de Ko pour un SPX complet), avant tout filtre. `history.csv` ne
garde que les agrégats : sans l'archive, impossible de rejouer une séance passée à un
autre horizon, ni de corriger une erreur de méthode autrement qu'en attendant que
l'historique se reconstitue.

```sh
python snapshots.py                # les relevés archivés
python snapshots.py SPX

python main.py SPX --replay snapshots/SPX/2026-08-12_1508.parquet --dte-max 7
python main.py SPX --replay snapshots/SPX/2026-08-12_1508.parquet --gamma-source published
python main.py SPCX --no-snapshot  # ne pas archiver
```

Un rejeu ne réarchive pas et ne consomme aucune requête réseau. Le format est parquet
si `pyarrow` est installé, sinon `csv.gz`.

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
ticker    dte     gamma              date       spot        GEX   zero gam   call w.    put w.
SPCX       30        iv  2026-08-03 16:00     114.85     -58.7M     137.51    160.00    100.00
SPCX       30        iv  2026-08-06 16:00     110.48    -147.8M     124.02    115.00    110.00
SPCX       30        iv  2026-08-10 16:00     134.10      +3.1M     132.02    150.00    150.00

variation sur 3 relevés :
  GEX              -58.7M ->        +3.1M       +61.8M   changement de signe
  le régime a changé de signe sur la période (gamma positif <-> négatif)
```

Le périmètre (`dte_max`) **et** la source de gamma sont enregistrés avec chaque ligne :
deux relevés du même jour qui n'en partagent pas ne sont pas comparables — le GEX change
de signe rien qu'en changeant d'horizon, et de plus de 10 % rien qu'en changeant de source.
L'affichage les sépare donc en sections plutôt que de les enchaîner dans une même série.
Un historique écrit par une version antérieure est migré automatiquement, sans décalage
de colonnes.

### Validation : le modèle tient-il ?

```sh
python validate.py            # tous les tickers
python validate.py SPCX
```

Le modèle avance trois affirmations vérifiables, et `validate.py` les mesure sur
l'historique :

1. **les mouvements sont plus amples en gamma négatif** — amplitude médiane par jour,
   comparée entre les deux régimes, puis selon la position vis-à-vis du zero gamma ;
2. **la volatilité réalisée dépasse l'implicite en gamma négatif** — l'amplitude de
   la séance suivante estimée à la Parkinson, `ln(H/L) / 2√(ln 2)`, rapportée à l'iv30
   du relevé. Un titre qui ouvre à 100, monte à 110 et retombe à 100 a bougé ; un
   rendement de clôture le compte pour zéro ;
3. **le prix bute sur les murs** — et la mesure distingue désormais deux choses que
   la seule clôture confondait :

```
   call wall  :  38 relevés, distance médiane +3.1%
                 touché en séance 71% du temps, tenu à la clôture 18%
                 -> rejeté après avoir été touché : 53% des séances
```

Un mur souvent touché mais rarement tenu, c'est exactement ce que le modèle prédit :
le prix y va, la couverture le repousse. En ne regardant que le cours suivant, un
aller-retour intraséance ressortait comme un mur respecté, indistinguable d'un mur
jamais approché.

Ces deux dernières mesures demandent le high/low et l'iv30 de la séance : elles
n'existent que pour les relevés enregistrés depuis, et le script le dit plutôt que
de calculer sur du vide.

En dessous de 20 intervalles, le script affiche les chiffres mais refuse d'en conclure
quoi que ce soit, et le dit. Il faut donc laisser l'historique s'accumuler — un relevé
par séance. Ce n'est pas un backtest de stratégie : on vérifie que la description du
terrain est exacte, pas qu'on peut en tirer de l'argent.

Deux garde-fous, sans lesquels la mesure se mesurait elle-même :

- **un périmètre à la fois.** Les relevés sont regroupés par (sous-jacent, `dte_max`,
  source de gamma, convention de temps). Les enchaîner classait le régime d'après un
  GEX qui changeait de signe rien qu'en changeant d'horizon — le cas du SPX cité plus
  haut, +70,8 Md sur toute la chaîne contre −1,1 Md sur le 0–7 DTE.
- **une séance = une observation.** Deux exécutions du même jour ne sont pas deux
  points : seule la dernière est retenue, et les intervalles de moins d'une demi-journée
  sont écartés. Normalisé en racine du temps, un mouvement réel de 0,2 % sur 20 minutes
  ressortait à **1,7 % par jour**, et cette valeur entrait telle quelle dans la médiane
  comparée entre régimes.

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
python -m pytest tests -q        # 147 tests, aucun accès réseau
```

Les greeks ne sont pas comparés à des valeurs codées en dur — celles-ci viendraient de la
même formule que le code et ne prouveraient rien. Charm et vanna sont recoupés par
**différences finies** sur le delta, le gamma Black-Scholes contre Black-76, et les parsers
contre des jeux construits depuis des paramètres connus (on vérifie qu'on retrouve le prix
du future, l'IV et le gamma injectés).

`tests/test_analysis.py` couvre le pipeline lui-même — filtre d'échéance, murs, profil,
zero gamma, source de gamma, convention de temps, archivage, graphiques. Cette logique
vivait dans `main()` : les tests précédents validaient les formules et les parsers, et
pas un seul des chiffres réellement affichés. Les chaînes d'essai y sont construites
depuis des paramètres connus, avec un open interest volontairement asymétrique entre
calls et puts — à OI égal, GEX net, charm et vanna sont identiquement nuls et la suite
passerait sur du vide.

La CI (`.github/workflows/tests.yml`) lance la suite sur Python 3.11 et 3.13 à chaque
push et chaque pull request.

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
python barchart_data.py E6U26 --expiry aug-26 --out barchart_6E.csv
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
python flow_tracker.py ES --interval 300 --contract-size 50   # le multiplicateur n'est pas toujours 100
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
