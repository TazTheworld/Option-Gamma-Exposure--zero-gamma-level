"""Historique quotidien du sous-jacent, via une API officielle à clé gratuite.

À quoi ça sert : `validate.py` mesure si les mouvements sont plus amples en gamma
négatif et si le prix respecte les murs. Sans série de prix, il n'a qu'un point
par exécution — il faut donc un mois de relevés avant qu'il accepte de conclure.
Avec l'historique, les séances passées existent déjà et la mesure est possible
dès le premier jour.

    export ALPHAVANTAGE_API_KEY=...       # ou TWELVEDATA_API_KEY, TIINGO_API_KEY
    python validate.py SPCX --prix

Trois fournisseurs sont supportés, tous avec une API documentée, des conditions
d'usage explicites et une clé gratuite. Le fournisseur est déduit de la clé
présente dans l'environnement, ou imposé par --fournisseur.

Pourquoi pas Stooq, qui ne demande aucune clé : depuis peu, il sert une épreuve
de calcul dont le seul objet est d'écarter les clients non-navigateurs. C'est un
refus d'accès automatisé, et on le respecte. Yahoo est écarté pour une raison
plus douce : endpoint non documenté, conditions floues, authentification déjà
changée une fois.
"""

import os

import pandas as pd
import requests

COLONNES = ["date", "open", "high", "low", "close", "volume"]

# Ces API couvrent les actions, rarement les indices. Un ETF n'EST PAS l'indice —
# il en suit la performance à quelques dixièmes près, dividendes et frais compris.
# La substitution est donc annoncée à l'écran, jamais faite en silence.
PROCURATIONS = {"SPX": "SPY", "NDX": "QQQ", "RUT": "IWM", "DJI": "DIA", "XSP": "SPY"}

FOURNISSEURS = {
    "alphavantage": {
        "cle": "ALPHAVANTAGE_API_KEY",
        "url": "https://www.alphavantage.co/query",
        "inscription": "https://www.alphavantage.co/support/#api-key",
    },
    "twelvedata": {
        "cle": "TWELVEDATA_API_KEY",
        "url": "https://api.twelvedata.com/time_series",
        "inscription": "https://twelvedata.com/pricing",
    },
    "tiingo": {
        "cle": "TIINGO_API_KEY",
        "url": "https://api.tiingo.com/tiingo/daily/{ticker}/prices",
        "inscription": "https://www.tiingo.com/",
    },
}


def symbole(ticker):
    """(symbole à demander, procuration utilisée ou None)."""
    propre = str(ticker).upper().lstrip("_^")
    return PROCURATIONS.get(propre, propre), PROCURATIONS.get(propre)


def choisir_fournisseur(nom=None):
    """Fournisseur explicite, ou le premier dont la clé est dans l'environnement."""
    if nom:
        if nom not in FOURNISSEURS:
            raise ValueError(f"fournisseur inconnu : {nom!r} "
                             f"(attendu : {', '.join(FOURNISSEURS)})")
        cle = os.environ.get(FOURNISSEURS[nom]["cle"])
        if not cle:
            raise ValueError(f"{FOURNISSEURS[nom]['cle']} absente de l'environnement. "
                             f"Clé gratuite : {FOURNISSEURS[nom]['inscription']}")
        return nom, cle

    for candidat, info in FOURNISSEURS.items():
        cle = os.environ.get(info["cle"])
        if cle:
            return candidat, cle

    variables = ", ".join(f["cle"] for f in FOURNISSEURS.values())
    raise ValueError(
        f"Aucune clé d'historique de prix trouvée. Définis l'une de : {variables}.\n"
        + "\n".join(f"  {nom:<13} {info['inscription']}" for nom, info in FOURNISSEURS.items()))


# ---=== Normalisation ===---

def _trame(lignes):
    """Lignes hétérogènes -> trame commune, triée du plus ancien au plus récent."""
    df = pd.DataFrame(lignes, columns=COLONNES)
    df["date"] = pd.to_datetime(df["date"]).dt.normalize()
    for colonne in COLONNES[1:]:
        df[colonne] = pd.to_numeric(df[colonne], errors="coerce")
    df = df.dropna(subset=["date", "close"])
    return df.sort_values("date").drop_duplicates("date").reset_index(drop=True)


def _alphavantage(session, url, sym, cle, timeout):
    reponse = session.get(url, timeout=timeout, params={
        "function": "TIME_SERIES_DAILY", "symbol": sym,
        "outputsize": "full", "apikey": cle})
    reponse.raise_for_status()
    charge = reponse.json()
    series = charge.get("Time Series (Daily)")
    if not series:
        # Le quota et le symbole inconnu passent tous deux par un 200 : le message
        # est dans le corps, et l'ignorer donnerait une série vide sans explication.
        message = (charge.get("Error Message") or charge.get("Note")
                   or charge.get("Information") or str(charge)[:200])
        raise ValueError(f"Alpha Vantage : {message}")
    return _trame([{"date": jour, "open": v["1. open"], "high": v["2. high"],
                    "low": v["3. low"], "close": v["4. close"], "volume": v["5. volume"]}
                   for jour, v in series.items()])


def _twelvedata(session, url, sym, cle, timeout):
    reponse = session.get(url, timeout=timeout, params={
        "symbol": sym, "interval": "1day", "outputsize": 5000, "apikey": cle})
    charge = reponse.json()
    if charge.get("status") == "error" or "values" not in charge:
        raise ValueError(f"Twelve Data : {charge.get('message', str(charge)[:200])}")
    return _trame([{"date": v["datetime"], "open": v.get("open"), "high": v.get("high"),
                    "low": v.get("low"), "close": v.get("close"), "volume": v.get("volume")}
                   for v in charge["values"]])


def _tiingo(session, url, sym, cle, timeout):
    reponse = session.get(url.format(ticker=sym.lower()), timeout=timeout,
                          params={"token": cle, "startDate": "2015-01-01"})
    if reponse.status_code != 200:
        raise ValueError(f"Tiingo : HTTP {reponse.status_code} — {reponse.text[:200]}")
    charge = reponse.json()
    if not isinstance(charge, list) or not charge:
        raise ValueError(f"Tiingo : réponse inattendue — {str(charge)[:200]}")
    return _trame([{"date": v["date"], "open": v.get("open"), "high": v.get("high"),
                    "low": v.get("low"), "close": v.get("close"), "volume": v.get("volume")}
                   for v in charge])


_LECTEURS = {"alphavantage": _alphavantage, "twelvedata": _twelvedata, "tiingo": _tiingo}


def fetch_ohlcv(ticker, fournisseur=None, cle=None, timeout=30):
    """Séances quotidiennes d'un sous-jacent -> (trame, fournisseur, procuration).

    La trame porte date, open, high, low, close, volume, du plus ancien au plus
    récent. `procuration` vaut le symbole substitué quand le ticker est un indice
    non couvert, None sinon.
    """
    nom, cle_trouvee = choisir_fournisseur(fournisseur)
    cle = cle or cle_trouvee
    sym, procuration = symbole(ticker)

    session = requests.Session()
    session.headers.update({"User-Agent": "gamma-exposure research"})
    trame = _LECTEURS[nom](session, FOURNISSEURS[nom]["url"], sym, cle, timeout)
    if trame.empty:
        raise ValueError(f"Aucune séance renvoyée pour {sym} par {nom}.")
    return trame, nom, procuration


if __name__ == "__main__":
    import sys

    ticker = sys.argv[1] if len(sys.argv) > 1 else "SPCX"
    try:
        seances, nom, procuration = fetch_ohlcv(ticker)
    except ValueError as err:
        raise SystemExit(f"Erreur : {err}")
    if procuration:
        print(f"ATTENTION : {ticker} est un indice ; l'historique vient de {procuration}, "
              f"qui le suit sans l'égaler.")
    print(f"{ticker} via {nom} : {len(seances)} séances, "
          f"du {seances.date.min():%Y-%m-%d} au {seances.date.max():%Y-%m-%d}")
    print(seances.tail())
