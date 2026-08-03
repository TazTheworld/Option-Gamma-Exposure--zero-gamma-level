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

### Source de données

`cboe_data.py` interroge `https://cdn.cboe.com/api/global/delayed_quotes/options/{TICKER}.json` :
chaîne complète (strike, expiration, OI, IV, gamma), gratuite, sans inscription, en différé.
Le module expose aussi `load_from_csv()` pour les exports du site CBOE et `to_cboe_csv()`
pour réécrire une chaîne au format CSV historique.

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
