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
