<h1 align="center">Option Gamma Exposure — zero gamma level 👋</h1>
<p>
  <img alt="Version" src="https://img.shields.io/badge/version-2.0-blue.svg?cacheSeconds=2592000" />
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-orange.svg" />
  <a href="#" target="_blank">
    <img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-yellow.svg" />
  </a>
</p>

> Où le marché bute, et pourquoi. Gamma exposure et zero gamma level sur les options du
> Nasdaq-100 (NQ, CME), rafraîchis pendant la séance depuis Interactive Brokers.

### 🏠 [Homepage](https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level)

## Ce que ça mesure

Les teneurs de marché qui vendent des options doivent se couvrir, et cette couverture
laisse une trace. Selon l'endroit où le prix se trouve, elle **amortit** les mouvements
ou les **amplifie** — et le point de bascule se calcule.

| | |
|---|---|
| **Gamma exposure (GEX)** | les dollars que les dealers doivent échanger pour chaque mouvement de 1 % |
| **Zero gamma level** | le niveau où leur couverture change de sens : au-dessus elle freine, en dessous elle accélère |
| **Call wall / Put wall** | les strikes où le gamma se concentre — là où le prix bute, et souvent rebrousse |
| **Charm** | le delta que le temps qui passe leur fait gagner ou perdre, à prix constant |
| **Vanna** | le même, mais provoqué par un choc de volatilité |

Ce n'est pas une stratégie. C'est une description du terrain : où la couverture pèse,
et dans quel sens.

## Ce que ça donne

```
NQ | sous-jacent 29,164.75 | 210 strikes / 3 échéances (<= 3j) | 2026-08-26 13:14
   | contrat x20 | gamma iv | T heures | vol sticky-strike
Total GEX  : -326.74 millions $ / mouvement de 1%
Zero Gamma : 29,275.48
Call Wall  :    29,800.00 (gamma)      29,800.00 (open interest)
Put Wall   :    29,000.00 (gamma)      28,500.00 (open interest)
Max Pain   :    29,270.00 (échéance la plus proche)
Charm      : -713.89 millions $ de delta / jour
Vanna      : +40.01 millions $ de delta / point de vol
Delta      : +4.87 milliards $ de sous-jacent
Vega       : -96.78 milliers $ / point de vol
Thêta      : -158.57 milliers $ / jour

Attention : les échéances à 0-1 jour portent 76% du GEX, 84% du charm. Leurs greeks
sont instables sur des données différées — compare avec --dte-min 2 avant de conclure.
```

L'en-tête n'est pas décoratif : il rappelle **sous quelles hypothèses** le chiffre a été
produit. Deux relevés qui ne partagent pas le même horizon, la même source de gamma ou la
même mesure du temps ne sont pas comparables — le GEX change de signe rien qu'en changeant
d'horizon.

## Démarrer

```sh
git clone https://github.com/TazTheworld/Option-Gamma-Exposure--zero-gamma-level.git
cargo build --release --manifest-path options-rs/Cargo.toml
```

Trois binaires en sortent. Le premier acquiert, les deux autres lisent :

```sh
gex-collector NQ       # acquiert : balaie la chaîne, entretient un relevé sur disque
gex NQ                 # lit : un rapport de séance
gex NQ --watch 30s     # relu toutes les 30 secondes
gex-web NQ             # lit : l'écran de séance, sur http://127.0.0.1:8787
```

Le collecteur exige **TWS ou IB Gateway** en fonctionnement, avec l'API activée. Le mode
différé suffit et ne demande aucun abonnement — il décale les données d'un quart d'heure,
ce qui est écrit à l'écran plutôt que subi.

Le lecteur, lui, ne va jamais chercher de données. C'est cette séparation qui lui permet
de tourner pendant que le collecteur encaisse le redémarrage quotidien d'IB, et qui fait
qu'une séance passée se rejoue avec exactement le même code qu'une séance vivante.

## Ce que le collecteur écrit

```
snapshots/NQ/
  courant.parquet    le relevé vivant, réécrit toutes les 15 s
  barres.parquet     les chandeliers d'une minute
  niveaux.parquet    zero gamma, murs, GEX, charm, vanna — un point par minute
```

Les deux séries existent parce que **c'est la dérive qui porte l'information** : un zero
gamma à 29 400 ne dit rien seul ; le voir monter de 29 200 pendant que le prix s'en
approche, si. Elles sont bornées à trente jours glissants.

> Quand le marché ne cote pas — la nuit américaine — la série des niveaux **n'écrit
> rien** plutôt qu'un point à zéro. Une ligne plate se lirait comme « le gamma est nul »
> là où la donnée dit « je ne cote pas ». Les barres, elles, continuent : le future se
> traite la nuit.

## L'écran de séance

```sh
gex-web NQ
```

Sept nombres dans un terminal sont exacts et illisibles d'un coup d'œil : un zero gamma à
29 275 ne dit rien tant qu'on ne voit pas où le prix se tient par rapport à lui, ni depuis
quand il dérive. L'écran met les niveaux **sur** le prix.

Sur le prix : le **zero gamma** avec sa trace et **les autres bascules** du régime, les
**murs** — ceux du gamma et ceux de l'open interest, qui répondent à deux questions
différentes et divergent souvent —, le **max pain**, la bande **±1σ implicite** d'ici
l'échéance, et le **régime peint en fond** : vert là où la couverture amortirait, rouge là
où elle amplifierait. Ce fond n'est pas un niveau, c'est une carte — il dit ce qui
arriverait *si* le prix allait là.

À droite, le **GEX par strike**, en barres posées à la hauteur exacte de leur strike sur
l'axe des prix : quand le cours bouge, le profil bouge avec.

En bas, les séries temporelles, **chacune dans sa bande**, groupées par ce qu'elles
disent : le **flux** que la couverture impose (GEX, charm, vanna), la **position** du book
(delta, vega, thêta), et la **volatilité** (IV ATM, skew). Elles ne partagent pas d'axe —
ce sont huit unités différentes, et un croisement entre deux d'entre elles ne voudrait rien
dire.

> Flux et position ne sont pas la même chose. Le GEX, le charm et le vanna disent ce que
> les teneurs de marché doivent **acheter ou vendre** ; le delta, le vega et le thêta
> disent ce que leur position **est**. Le thêta en particulier n'engendre aucun flux de
> couverture : il ne dit pas quoi faire, il dit ce que ne rien faire coûte.

Il **ne calcule rien** : il lit les trois fichiers du collecteur et les traduit. Refaire
l'analyse à chaque requête dupliquerait le moteur dans un second chemin, et deux chemins
finissent par diverger — l'écran montrerait alors autre chose que le lecteur.

Il n'écoute que sur `127.0.0.1` : ces relevés sont à toi. La bibliothèque de graphiques
est servie depuis le disque, pas depuis un CDN, pour que l'écran marche sans réseau.

> L'horodatage du dernier relevé est affiché **en permanence**, et vieillit visiblement
> quand le collecteur s'arrête. Un écran qui a l'air vivant alors qu'il est figé est pire
> qu'un écran vide.

## Les choix qui changent le chiffre

Un GEX n'existe pas dans l'absolu. Quatre décisions le déplacent, parfois de plus de
100 %, et chacune est un drapeau explicite plutôt qu'un défaut caché.

### D'où vient le gamma (`--gamma-source`)

IB publie un gamma par contrat, et il se recalcule aussi en Black-76 depuis la
volatilité implicite. **Une seule source alimente tout le pipeline**, et l'en-tête dit
laquelle.

Le défaut est `iv`, seul choix cohérent de bout en bout : à un niveau de spot
hypothétique, aucun gamma publié n'existe, donc le profil ne peut être que recalculé.
Avec `published`, le total et le zero gamma viennent forcément d'estimateurs
différents — l'écart est alors affiché dès qu'il dépasse 5 %.

Les deux se rejoignent sur les horizons courants et divergent aux extrêmes : sur les
0-1 DTE, où le gamma explose et dépend du spot à la minute que des données différées ne
donnent pas ; et sur les échéances lointaines, où l'hypothèse `r = q = 0` cesse d'être
neutre.

### L'horizon d'échéance (`--dte-max`)

Une chaîne complète porte des échéances jusqu'à plusieurs trimestres. Prises en bloc, les
lointaines — strikes ronds à très gros open interest — dominent les murs et tirent le zero
gamma, alors qu'elles ne produisent aucun flux de couverture à court terme.

L'effet n'est pas marginal : sur un indice, la chaîne entière et le 0–7 DTE peuvent
donner des GEX de **signes opposés** pour la même séance. Le défaut est 30 jours.

### La mesure du temps (`--time-convention`)

Le défaut est `heures` : le temps réel restant jusqu'au règlement servi par IB, rapporté
à 365 jours. C'est ce qui rend le gamma recalculé et le gamma publié comparables — ils se
superposent presque exactement.

`bourse` restaure la convention du script de référence, jours ouvrés / 262 avec un
plancher à un jour. Ce plancher surestime lourdement les 0DTE — un 0DTE à 10 h du matin,
c'est 0,23 jour, pas 1 — et le gamma variant en 1/√T, l'écart est massif. Il est là pour
reproduire, pas pour être utilisé.

### Ce que devient la volatilité (`--vol-regime`)

Quand le spot bouge, l'IV suit-elle le strike ou la monnaie ? Les deux hypothèses encadrent
la réalité. Le défaut, `sticky-strike`, est celui de la littérature ;
`sticky-moneyness` remodèle les ailes du profil sans déplacer un zero gamma proche du spot.

## Zero gamma : quel croisement ?

Le profil peut repasser par zéro plusieurs fois dès que les ailes sont bruyantes. C'est le
croisement **le plus proche du spot** qui est retenu — celui qui délimite le régime où le
marché se trouve effectivement. Les autres sont **nommés**, pas seulement comptés :

```
Attention : le profil croise zéro 2 fois — 29,196.62 / 36,508.00. Le niveau retenu est
le plus proche du spot (29,196.62) ;
le régime n'est pas une simple bascule au-dessus / en dessous.
```

Sur l'écran de séance, les autres bascules sont tracées en trait fin sous le zero gamma
retenu. Ce sont les croisements du profil **peint en fond**, donc les endroits exacts où
le dégradé change de couleur : les traits nomment ce que le fond montre déjà.

## Max pain

Le strike où l'ensemble des options en circulation vaudrait le moins **au règlement** :
pour chaque strike candidat `K`, on somme la valeur intrinsèque de tous les contrats
ouverts si le sous-jacent réglait là, et on garde le minimum.

Il est calculé **sur une seule échéance**, la plus proche. La valeur intrinsèque ne se
cristallise qu'au règlement, et deux échéances règlent deux jours différents : les
additionner supposerait que le prix est le même les deux jours.

C'est une **description de l'open interest**, au même titre que les murs — pas une
prévision. La théorie du « pinning » autour du max pain reste contestée, et **rien dans ce
dépôt ne la vérifie** : `gex --valider` confronte trois affirmations aux séances observées,
et celle-ci n'en fait pas partie.

> Ne pas le confondre avec le zero gamma. Le zero gamma vient des **greeks** et bouge quand
> le prix bouge ; le max pain vient de l'**open interest seul** et ne bouge que quand
> quelqu'un ouvre ou ferme des positions.

## Le modèle tient-il ?

```sh
gex --valider
```

Chaque relevé s'ajoute à `history.csv`, et `--valider` confronte trois affirmations à ce
qui s'est réellement passé : les mouvements sont-ils plus amples en gamma négatif ? La
position vis-à-vis du zero gamma décide-t-elle du régime ? Le prix bute-t-il sur les murs ?

Deux garde-fous, sans lesquels la mesure se mesurerait elle-même. **Un périmètre à la
fois** — les enchaîner classerait le régime d'après un GEX qui change de signe rien qu'en
changeant d'horizon. **Une séance = une observation** — normalisé en racine du temps, un
mouvement réel de 0,2 % sur vingt minutes ressortait à 1,7 % par jour.

En dessous de vingt observations, les chiffres s'affichent mais aucune conclusion n'est
tirée, et c'est dit. Ce n'est pas un backtest de stratégie : on vérifie que la description
du terrain est exacte, pas qu'on peut en tirer de l'argent.

## Source de données

**Interactive Brokers**, et elle seule — TWS ou Gateway, options sur futures CME, forfait,
temps réel ou différé.

IB apporte trois choses ensemble, ce qui est rare : le temps réel, un tarif forfaitaire,
et un gamma **publié** — c'est lui qui permet de confronter deux estimateurs sur du
future plutôt que d'en croire un sur parole.

**Lecture seule.** Le collecteur énumère des contrats et souscrit à des cotations. Aucun
ordre n'est passé nulle part.

## Ce que ça n'est pas

- **Pas du temps réel** sans abonnement CME : le différé décale de quinze minutes.
- **Pas de graphiques** : la sortie est du texte. Le profil complet et le détail par
  strike restent dans la sortie du moteur, pour qui voudrait les tracer autrement.
- **Pas un signal d'achat.** Le GEX décrit une contrainte de couverture, pas une direction.

## Sous le capot

**Rust de bout en bout**, collecteur compris, en cinq crates. Aucune autre dépendance :
ni Python, ni environnement virtuel, ni paquet à installer. La discipline y est
structurelle plutôt que conventionnelle — la crate qui produit les chiffres ne déclare
aucune dépendance capable d'ouvrir un fichier, un socket, ni même de lire l'horloge, et
un calcul qui dépendrait de l'heure courante ne compile pas.

L'architecture, les conventions, les tests et les pièges d'Interactive Brokers sont dans
[`AGENTS.md`](AGENTS.md) et [`options-rs/README.md`](options-rs/README.md).

Le calcul s'appuie sur le script de https://perfiliev.com/author/perfiliev/.

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
