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
NQ | sous-jacent 29,305.75 | 142 strikes / 3 échéances (<= 30j) | 2026-08-25 20:30
   | contrat x20 | gamma iv | T heures | vol sticky-strike
Total GEX  : -357.42 millions $ / mouvement de 1%
Zero Gamma : n/a
Call Wall  :          n/a (gamma)            n/a (open interest)
Put Wall   :    29,200.00 (gamma)      28,750.00 (open interest)
Charm      : -15.93 millions $ de delta / jour
Vanna      : +3.18 millions $ de delta / point de vol

Attention : les échéances à 0-1 jour portent 40% du GEX, 3% du charm. Leurs greeks
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

Deux binaires en sortent, et ils tournent dans deux terminaux :

```sh
gex-collector NQ       # acquiert : balaie la chaîne, entretient un relevé sur disque
gex NQ                 # lit : un rapport de séance
gex NQ --watch 30s     # relu toutes les 30 secondes
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

## Les choix qui changent le chiffre

Un GEX n'existe pas dans l'absolu. Quatre décisions le déplacent, parfois de plus de
100 %, et chacune est un drapeau explicite plutôt qu'un défaut caché.

### D'où vient le gamma (`--gamma-source`)

IB publie un gamma par contrat, et on peut aussi le recalculer en Black-76 depuis la
volatilité implicite. Le script d'origine **mélangeait les deux sans le dire** : le total
prenait le gamma publié pendant que le profil — donc le zero gamma affiché juste en
dessous — recalculait depuis l'IV.

Le défaut est `iv`, seul choix cohérent de bout en bout : à un niveau de spot
hypothétique, aucun gamma publié n'existe. Avec `published`, l'écart entre les deux est
affiché dès qu'il dépasse 5 %.

Relevé du SPX du 12 août 2026, une fois la mesure du temps corrigée :

| horizon | gamma publié | recalculé | écart |
|---|---|---|---|
| ≤ 1 j | +5,06 Md | +5,64 Md | +11,4 % |
| ≤ 7 j | +15,37 Md | +15,62 Md | +1,7 % |
| ≤ 30 j | +39,89 Md | +39,46 Md | −1,1 % |
| toute la chaîne | +87,92 Md | +76,96 Md | −12,5 % |

L'écart subsiste là où on l'attend : sur les 0-1 DTE, où le gamma explose et dépend du
spot à la minute ; et sur les LEAPS, où l'hypothèse `r = q = 0` cesse d'être neutre.

### L'horizon d'échéance (`--dte-max`)

Une chaîne complète porte des échéances jusqu'à plusieurs trimestres. Prises en bloc, les
lointaines — strikes ronds à très gros open interest — dominent les murs et tirent le zero
gamma, alors qu'elles ne produisent aucun flux de couverture à court terme.

Sur le SPX du 10 août 2026, la chaîne entière donnait **+70,8 Md** de GEX quand le 0–7 DTE
donnait **−1,1 Md** : deux régimes opposés, pour la même séance. Le défaut est 30 jours.

### La mesure du temps (`--time-convention`)

Le script de référence compte en jours ouvrés / 262 avec un **plancher à un jour** pour les
0DTE. Ce plancher les surestime lourdement — un 0DTE à 10 h du matin, c'est 0,23 jour, pas
1 — et le gamma variant en 1/√T, l'erreur est massive.

Le défaut est `heures` : le temps réel restant jusqu'au règlement, rapporté à 365 jours.
Mesuré sur le SPX, le rapport gamma recalculé / gamma publié passe d'une médiane de 1,134
à 1,000.

### Ce que devient la volatilité (`--vol-regime`)

Quand le spot bouge, l'IV suit-elle le strike ou la monnaie ? Les deux hypothèses encadrent
la réalité. Le défaut, `sticky-strike`, est celui de la littérature ;
`sticky-moneyness` remodèle les ailes du profil sans déplacer un zero gamma proche du spot.

## Zero gamma : quel croisement ?

Le profil peut repasser par zéro plusieurs fois dès que les ailes sont bruyantes. C'est le
croisement **le plus proche du spot** qui est retenu — celui qui délimite le régime où le
marché se trouve effectivement. Les autres sont signalés :

```
Attention : le profil croise zéro 2 fois (également en 7,412.30). Le niveau retenu est
le plus proche du spot ; le régime n'est pas une simple bascule au-dessus / en dessous.
```

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

Le CBOE, le CME, Databento et Barchart ont été retirés : tous ne servaient que le règlement
de la veille, et aucun ne publiait le gamma. IB apporte les trois choses qu'aucun n'avait —
le temps réel, un tarif forfaitaire, et un gamma **publié**, ce qui permet enfin de
confronter deux estimateurs sur du future.

**Lecture seule.** Le collecteur énumère des contrats et souscrit à des cotations. Aucun
ordre n'est passé nulle part.

## Ce que ça n'est pas

- **Pas du temps réel** sans abonnement CME : le différé décale de quinze minutes.
- **Pas de graphiques** : le portage en Rust ne les a pas repris. Le profil complet reste
  dans la sortie du moteur, pour qui voudrait le tracer autrement.
- **Pas un signal d'achat.** Le GEX décrit une contrainte de couverture, pas une direction.

## Sous le capot

Tout est en **Rust** (`options-rs/`), collecteur compris, et la discipline du dépôt y est
devenue structurelle : la crate qui produit les chiffres ne déclare aucune dépendance
capable d'ouvrir un fichier, un socket, ni même de lire l'horloge — un calcul qui
dépendrait de l'heure courante ne compile pas.

Le seul Python restant est `sec_data.py`, qui lit les déclarations d'initiés auprès de la
SEC et n'a jamais eu de rapport avec les options.

L'architecture, les conventions, les tests et les pièges rencontrés sont dans
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
