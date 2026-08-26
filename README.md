<h1 align="center">Bienvenue pour l'Option Gamma Exposure et zero gamma level 👋</h1>
<p>
  <img alt="Version" src="https://img.shields.io/badge/version-1.1-blue.svg?cacheSeconds=2592000" />
  <a href="#" target="_blank">
    <img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-yellow.svg" />
  </a>
</p>

> Gamma exposure et zero gamma level sur les options du Nasdaq-100 (NQ, CME), rafraîchis
> pendant la séance. Un collecteur Interactive Brokers entretient un relevé sur disque, un
> lecteur le calcule — les deux tournent séparément, pour que le lecteur survive au
> redémarrage quotidien d'IB. Le calcul s'appuie sur le script de
> https://perfiliev.com/author/perfiliev/.

**Tout est en Rust** (`options-rs/`), collecteur compris. La discipline du dépôt y est
devenue structurelle plutôt que conventionnelle : la crate qui produit les chiffres ne
déclare aucune dépendance capable d'ouvrir un fichier, un socket, ni même de lire
l'horloge — un calcul qui dépendrait de l'heure courante ne compile pas.

Le seul Python restant est `sec_data.py`, qui lit les déclarations d'initiés auprès de
la SEC et n'a jamais eu de rapport avec les options.

### 🏠 [Homepage](https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level)

## Installation

```sh
git clone https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level.git
python -m venv venv
venv\Scripts\activate
pip install -r requirements.txt
```

```sh
cargo build --release --manifest-path options-rs/Cargo.toml
```

Les deux binaires atterrissent dans `options-rs/target/release/` : `gex-collector` et
`gex`. `requirements.txt` ne sert plus qu'à `sec_data.py` — pandas et requests.

```sh
pip install -e ".[ib]"           # collecteur Interactive Brokers (ib_async)
pip install -e ".[snapshots]"    # archivage en parquet plutôt qu'en csv.gz
pip install -e ".[dev]"          # tests
```

Le collecteur exige en plus **TWS ou IB Gateway** en fonctionnement, avec l'API
activée. Le mode différé suffit et ne demande aucun abonnement.

## Organisation

| Module | Rôle |
|---|---|
| Crate | Rôle |
|---|---|
| `gex-core` | Black-76, greeks, expositions, murs, profil, zero gamma — **aucune E/S possible** |
| `gex-ib` | la source : décisions de collecte, et la couche réseau TWS |
| `gex-store` | relevés parquet, historique, validation du modèle |
| `gex-collector` | le démon : socle quotidien, vif entretenu |
| `gex-cli` | le binaire `gex` : rapport de séance, `--watch`, `--valider` |

Le détail de chaque crate est dans `options-rs/README.md`.

Le format parquet n'a pas changé pendant le portage : les relevés archivés du temps de
Python se rejouent tels quels.

La règle « tout ce qui produit un chiffre est testable sans réseau » était une
convention qu'une distraction suffisait à enfreindre. En Rust elle est structurelle :
`gex-core` n'a aucune dépendance capable d'ouvrir un fichier ou un socket, et `chrono`
y est déclaré sans sa feature `clock`, si bien qu'`Utc::now()` **n'existe pas**. Seul
le collecteur lit l'horloge.

## Utilisation

Deux programmes, dans deux terminaux. Le collecteur écrit, le lecteur lit :

```sh
gex-collector NQ                          # le collecteur : entretient le relevé
gex NQ                                    # le lecteur : un passage sur le courant
gex NQ --watch 30s                        # relu toutes les 30 s
gex NQ --range 0.35
```

Le lecteur ne va jamais chercher de données. C'est cette séparation qui lui permet de
tourner pendant que le collecteur encaisse le redémarrage quotidien d'IB, et qui fait
qu'une séance passée se rejoue avec exactement le même code qu'une séance vivante.

Le script affiche le Total GEX, le **Zero Gamma Level**, le Call Wall et le Put Wall,

> **Les graphiques ont été retirés.** `plots.py` traçait quatre PNG ; le portage en
> Rust ne les a pas repris, et le choix est assumé plutôt que subi — les chiffres et
> leurs avertissements portent l'essentiel, et `plotters` aurait coûté plus cher que
> ce qu'il rapportait. Le profil complet reste disponible dans la sortie de
> `gex-core`, pour qui voudrait le tracer autrement.

### D'où vient le gamma (`--gamma-source`)

IB publie un gamma par contrat, et on peut aussi le recalculer en Black-76 depuis la
volatilité implicite. Ni le CME ni Databento ne le publiaient, si bien que cette
confrontation n'était possible que sur des actions ; elle l'est désormais sur du
future. Le script **mélangeait les deux sans le dire** : le
Total GEX prenait le gamma publié pendant que le profil — donc le Zero Gamma affiché
juste en dessous — recalculait depuis l'IV. Deux estimateurs pour deux chiffres
présentés comme cohérents.

Une seule source alimente désormais tout le pipeline, rappelée dans l'en-tête comme
dans l'en-tête :

```sh
gex NQ                             # --gamma-source iv (défaut)
gex NQ --gamma-source published    # le gamma tel que publié par IB
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

Un montant de couverture nu ne dit rien : rapporté au volume échangé du jour, si.

> **Indisponible sur futures.** Ce contexte de séance (OHLCV, iv30) venait du payload
> CBOE. Aucune source sur futures ne le publie, donc la ligne `GEX/volume` ne
> s'affiche plus — elle est absente plutôt que fausse. Le mécanisme et le schéma
> d'historique restent en place pour que les relevés déjà écrits restent lisibles.

L'affichage, du temps où la donnée existait :

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

Une chaîne complète porte plusieurs années d'échéances — les trimestrielles du NQ
courent bien au-delà de l'horizon utile, et la mesure ci-dessous a été faite sur le
SPX, où l'effet est le plus net : jusqu'à 2031. Prises
en bloc, les LEAPS écrasent l'analyse : leurs strikes ronds concentrent un OI énorme
mais purement spéculatif, qui ne produit aucun flux de hedging à court terme. Sur le
SPX du 10 août 2026, la chaîne complète donnait un GEX de **+70,8 Md$** (régime de
compression) alors que le 0–7 DTE — celui qui pilote réellement le hedging du jour —
donnait **−1,1 Md$**, soit le régime inverse.

`--dte-max` ne retient donc que les échéances proches, **défaut 30 jours calendaires** :

```sh
gex NQ                    # 30 jours (défaut)
gex NQ --dte-max 7        # semaine en cours
gex NQ --toutes-echeances      # toute la chaîne collectée
```

Le filtre s'applique avant tout calcul : murs, profil de gamma et zero gamma portent
toujours sur le même périmètre, rappelé dans l'en-tête.
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
signalés dans la sortie :

```
Attention : le profil croise zéro 2 fois (également en 7,412.30). Le niveau retenu est
le plus proche du spot ; le régime n'est pas une simple bascule au-dessus / en dessous.
```

### Ce que devient la volatilité quand le spot bouge (`--vol-regime`)

Le profil de gamma évalue l'exposition à des niveaux de spot hypothétiques. Reste
à décider ce que devient l'IV en chemin, et les deux réponses encadrent la réalité :

```sh
gex NQ                                  # sticky-strike (défaut)
gex NQ --vol-regime sticky-moneyness
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
gex NQ --watch 30s --watch-duration 6h
```

L'open interest ne bouge qu'une fois par jour : en séance, seuls le spot et l'IV
changent. Le zero gamma ne se déplace donc pas beaucoup, mais la **distance** du
prix à ce niveau, elle, se referme ou s'ouvre — et c'est elle qui décide du régime.

```
  depuis le relevé précédent : spot +0.18, zero gamma +1.06, distance au zero gamma +9.72%
```

Un passage n'écrit dans l'historique que si le relevé a réellement avancé : le
collecteur réécrit le courant toutes les quinze secondes, donc un `--watch` plus
pressé que lui relit le même fichier, et le réenregistrer n'ajouterait qu'une ligne
identique.

### Le relevé courant, et les archives

```sh
gex NQ                          # lit snapshots/NQ/courant.parquet
gex NQ --watch 30s              # et le relit toutes les 30 s
```

`--replay` ouvre une archive horodatée, qui ne bougera plus. Sans lui, `gex` ouvre le
**relevé courant**, à chemin fixe, que le collecteur réécrit au fil de la séance. Il
n'y a qu'une source, donc exiger un drapeau pour la nommer ferait un drapeau
obligatoire, donc inutile.
D'où deux différences : `--watch` est permis sur le courant alors qu'il reste
interdit sur une archive, et `snapshots.lister()` exclut le courant — un fichier
qui change sous les pieds n'a rien à faire dans un historique de validation.

Ce qui l'écrit : `gex-collector NQ`, qui exige TWS ou Gateway avec l'API activée. Il balaie la chaîne entière une fois par journée de compensation — l'open
interest n'est publié qu'une fois par jour, le relire en séance coûterait treize
minutes pour le même chiffre — puis entretient en continu les contrats qui portent
le gamma, et réécrit le courant toutes les quinze secondes.

### Rejouer une séance (`--replay`)

```sh
gex NQ --replay snapshots/NQ/2026-08-26_0659.parquet --dte-max 7
```

`--replay` ouvre n'importe quel relevé au lieu du courant, et permet de recalculer
une séance passée sous un autre horizon. `history.csv` ne garde que les agrégats :
sans l'archive de la chaîne **brute**, on ne peut ni changer d'horizon après coup,
ni corriger une erreur de méthode autrement qu'en attendant que l'historique se
reconstitue.

> **L'archivage est désactivé par défaut.** Il sert à la recherche, pas au suivi
> de séance, et les fichiers s'accumulent sans que rien ne les efface. Pour
> l'activer : `gex-collector NQ --archiver 900` (une archive tous les quarts
> d'heure).

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


### Historique

Chaque exécution ajoute une ligne à `history.csv` — sans quoi chaque analyse reste un
instantané et les séries n'existent nulle part.

```sh
cat history.csv                # l'historique brut, une ligne par relevé
gex NQ --no-history      # ne pas enregistrer
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
gex --valider                 # sur history.csv
gex --valider --history autre.csv
```

Le modèle avance trois affirmations vérifiables, et `gex --valider` les mesure sur
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

#### Une vraie série de prix (`--prix`)

Sans historique de prix, le mouvement est mesuré **d'un relevé au suivant** : il
saute les séances où rien n'a été lancé, et dépend de l'heure d'exécution. Il faut
donc une vingtaine de séances relevées à la main avant que le script accepte de
conclure.

Avec une série quotidienne, le relevé ne sert plus qu'à décrire le régime, et le
résultat se lit sur la **séance boursière qui suit** — du haut au bas et jusqu'à la
clôture :

```sh
export ALPHAVANTAGE_API_KEY=...        # ou TWELVEDATA_API_KEY, TIINGO_API_KEY
gex --valider NQ --prix
gex --valider NQ --prix --fournisseur tiingo
```

Trois fournisseurs, tous avec une API documentée et une clé gratuite ; celui dont
la clé est présente est retenu automatiquement. Sans clé, le script le dit et
retombe sur les relevés seuls — `--prix` n'est jamais bloquant.

`price_data.py` ne remplace pas le `spot` du relevé : c'est le prix auquel les murs
et le zero gamma ont été calculés, et le substituer rendrait les distances
incohérentes avec les niveaux qu'elles mesurent. Il n'ajoute que la séance suivante.

Deux réserves dites franchement. Ces API cotent les actions, rarement les indices, et
**pas les futures du tout** : il n'existe aucune série pour NQ, donc `--prix` ne sert
plus rien sur le seul produit désormais couvert, et la validation retombe sur la
mesure à la clôture. Le module est conservé tel quel, sa substitution d'indice par un
ETF comprise — annoncée à l'écran, jamais faite en silence — mais il attend une source
de prix pour futures. Et
**Stooq n'est pas utilisé** : il servait des CSV sans clé, mais sert désormais une
épreuve de calcul dont le seul objet est d'écarter les clients non-navigateurs.
C'est un refus d'accès automatisé, et on le respecte.

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
cd options-rs
cargo test --workspace                                  # 135 tests : tout le moteur
cargo clippy --workspace --all-targets -- -D warnings

cd .. && python -m pytest tests -q                      # 12 tests : sec_data
```

**Aucun test ne touche au réseau, ni à TWS.** C'est la contrainte qui structure tout le
reste, et le portage l'a rendue vérifiable par le compilateur plutôt que par la
discipline : `gex-core` ne déclare aucune dépendance capable d'ouvrir quoi que ce soit.

Les greeks ne sont pas comparés à des valeurs codées en dur — celles-ci viendraient de
la même formule que le code et ne prouveraient rien. Charm, vanna et gamma sont recoupés
par **différences finies** sur le delta et sur la prime. Les deux formules du gamma,
celle du call et celle du put, sont mathématiquement égales et doivent coïncider : leur
désaccord est le seul signal qu'une faute de frappe dans l'une produirait.

Les jeux d'essai sont **construits depuis des paramètres connus**, jamais figés depuis
une sortie observée — un tel test passe encore quand le calcul devient faux. Le test du
skew, par exemple, doit retrouver exactement le −0,15 qu'on a injecté.

Et `options-rs/fixtures/` garde un **relevé IB réel** avec les nombres que le moteur
Python en tirait. C'est l'oracle du portage : sans lui, les deux moteurs auraient pu se
tromper identiquement sans que rien ne le révèle. Le dépôt a déjà payé cette leçon une
fois, avec le multiplicateur du contrat.

Ce qui ne se teste pas hors ligne vit dans `options-rs/crates/gex-ib/examples/sonde.rs` :
une sonde manuelle qui confronte le client à un TWS vivant. Trois défauts n'ont été
trouvés que par elle — voir le README de `options-rs`.

Il n'y a pas d'intégration continue : les deux suites se lancent à la main, avant
de pousser.

### Source de données

| Module | Source | Accès |
|---|---|---|
| `ib_data.py` | Interactive Brokers (TWS / Gateway), options sur futures CME | forfait, temps réel ou différé |

Il n'y en a plus qu'une. Le CBOE, le CME, Databento et Barchart ont été retirés : tous
ne servaient que le règlement de la veille, et aucun ne publiait le gamma — `black76.py`
le recalculait faute de mieux. IB apporte les trois choses qu'aucun n'avait : le temps
réel, un tarif forfaitaire, et un gamma **publié**, ce qui permet enfin à
`--gamma-source` de confronter deux estimateurs sur du future.

Le prix comptant du future ne se cherche pas : IB le sert dans `modelGreeks.undPrice`,
donc dans chaque tick d'option. La parité call-put ne reste qu'un filet.

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
