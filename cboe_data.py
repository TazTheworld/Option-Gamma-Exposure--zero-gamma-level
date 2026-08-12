"""Récupération des chaînes d'options CBOE (delayed quotes, gratuit, sans clé API).

Deux sources possibles, qui renvoient toutes les deux le même triplet
(df, spotPrice, quoteDate) attendu par main.py :

  - fetch_chain(ticker) : API JSON publique du CBOE
  - load_from_csv(path) : fichier "spx_quotedata.csv" téléchargé sur
    https://www.cboe.com/delayed_quotes (ancien format utilisé par le projet)
"""

import io
import re
from datetime import datetime, timedelta

import pandas as pd
import requests

# Endpoint public, données différées, pas de clé nécessaire
JSON_URL = "https://cdn.cboe.com/api/global/delayed_quotes/options/{symbol}.json"

# Le CBOE préfixe les indices par un underscore (_SPX, _VIX...), pas les actions
INDEX_SYMBOLS = {"SPX", "SPXW", "VIX", "NDX", "RUT", "XSP", "DJX", "OEX"}

# Colonnes du format "large" (une ligne = un couple expiration/strike)
COLUMNS = [
    "ExpirationDate", "Calls", "CallLastSale", "CallNet", "CallBid", "CallAsk",
    "CallVol", "CallIV", "CallDelta", "CallGamma", "CallOpenInt", "StrikePrice",
    "Puts", "PutLastSale", "PutNet", "PutBid", "PutAsk", "PutVol", "PutIV",
    "PutDelta", "PutGamma", "PutOpenInt",
]

_OPTION_RE = re.compile(r"^(?P<root>.+?)(?P<exp>\d{6})(?P<cp>[CP])(?P<strike>\d{8})$")


def cboe_symbol(ticker):
    """SPCX -> SPCX, SPX -> _SPX (convention CBOE pour les indices)."""
    ticker = ticker.upper().lstrip("^")
    if ticker.startswith("_"):
        return ticker
    return "_" + ticker if ticker in INDEX_SYMBOLS else ticker


def fetch_json(ticker, timeout=30):
    """Payload brut de l'API CBOE.

    Point d'entrée réseau unique du projet : flow_tracker s'en sert aussi, pour
    que le traitement des erreurs et l'en-tête User-Agent ne divergent pas entre
    deux implémentations.
    """
    symbol = cboe_symbol(ticker)
    resp = requests.get(
        JSON_URL.format(symbol=symbol),
        timeout=timeout,
        headers={"User-Agent": "Mozilla/5.0 (gamma-exposure research)"},
    )
    # Le CDN renvoie 403 (et non 404) pour un symbole qui n'existe pas
    if resp.status_code in (403, 404):
        raise ValueError(
            f"Aucune chaîne d'options CBOE pour '{ticker}' (symbole testé : {symbol}). "
            "Vérifie le ticker, ou préfixe les indices d'un underscore (_SPX)."
        )
    resp.raise_for_status()
    return resp.json()


# Le payload porte la séance du sous-jacent et sa vol implicite 30 jours, en plus
# de la chaîne. Le projet les ignorait, alors qu'ils sont téléchargés à chaque
# appel — et sans eux le GEX n'est qu'un montant en dollars sans référence :
# 120 M$ de couverture ne veulent rien dire tant qu'on ne les rapporte pas au
# volume du jour. Le high et le low servent aussi à tester les murs sur le
# parcours réel de la séance plutôt que sur le seul cours de clôture.
MARCHE = ("open", "high", "low", "close", "prev_day_close", "volume",
          "iv30", "iv30_change", "price_change_percent")


def marche_depuis_payload(payload):
    """Contexte de séance du sous-jacent : OHLCV, iv30. Champs absents -> None."""
    data = payload["data"]
    contexte = {}
    for champ in MARCHE:
        valeur = data.get(champ)
        try:
            contexte[champ] = None if valeur is None else float(valeur)
        except (TypeError, ValueError):
            contexte[champ] = None
    prix = contexte.get("close") or float(data["current_price"])
    volume = contexte.get("volume")
    # Le volume en titres ne se compare pas d'un sous-jacent à l'autre ; en
    # dollars, si — et c'est la seule échelle à laquelle le GEX est lisible.
    contexte["dollar_volume"] = None if not volume else volume * prix
    return contexte


def fetch_marche(ticker, timeout=30):
    """Contexte de séance seul, sans assembler la chaîne."""
    return marche_depuis_payload(fetch_json(ticker, timeout))


def fetch_chain(ticker, timeout=30):
    """Télécharge la chaîne d'options d'un sous-jacent US.

    Renvoie (df, spotPrice, quoteDate). Pour obtenir aussi le contexte de séance
    sans payer un second téléchargement, utiliser fetch_chain_et_marche().
    """
    return fetch_chain_et_marche(ticker, timeout)[:3]


def fetch_chain_et_marche(ticker, timeout=30):
    """(df, spotPrice, quoteDate, marche) en un seul appel réseau."""
    payload = fetch_json(ticker, timeout)
    data = payload["data"]
    spot_price = float(data["current_price"])
    quote_date = pd.to_datetime(payload["timestamp"]).to_pydatetime().replace(tzinfo=None)

    rows = []
    for opt in data["options"]:
        parsed = _OPTION_RE.match(opt["option"])
        if parsed is None:  # symbole exotique : on ignore plutôt que de planter
            continue
        rows.append({
            "ExpirationDate": parsed["exp"],
            "StrikePrice": int(parsed["strike"]) / 1000.0,
            "cp": parsed["cp"],
            "LastSale": opt.get("last_trade_price"),
            "Net": opt.get("change"),
            "Bid": opt.get("bid"),
            "Ask": opt.get("ask"),
            "Vol": opt.get("volume"),
            "IV": opt.get("iv"),
            "Delta": opt.get("delta"),
            "Gamma": opt.get("gamma"),
            "OpenInt": opt.get("open_interest"),
        })

    if not rows:
        raise ValueError(f"Chaîne d'options vide pour '{ticker}'.")

    raw = pd.DataFrame(rows)
    raw["ExpirationDate"] = (
        pd.to_datetime(raw["ExpirationDate"], format="%y%m%d") + timedelta(hours=16)
    )

    # Passage du format long (1 ligne = 1 contrat) au format large attendu par main.py
    fields = ["LastSale", "Net", "Bid", "Ask", "Vol", "IV", "Delta", "Gamma", "OpenInt"]
    keys = ["ExpirationDate", "StrikePrice"]
    calls = raw[raw.cp == "C"].set_index(keys)[fields].add_prefix("Call")
    puts = raw[raw.cp == "P"].set_index(keys)[fields].add_prefix("Put")

    # Un même strike peut apparaître deux fois (contrats ajustés) : on somme l'OI
    calls = calls.groupby(level=keys).agg({c: ("sum" if c == "CallOpenInt" else "first")
                                           for c in calls.columns})
    puts = puts.groupby(level=keys).agg({c: ("sum" if c == "PutOpenInt" else "first")
                                         for c in puts.columns})

    df = calls.join(puts, how="outer").reset_index()
    df["Calls"] = ""
    df["Puts"] = ""
    df = df.reindex(columns=COLUMNS)
    return _clean(df), spot_price, quote_date, marche_depuis_payload(payload)


def load_from_csv(filename):
    """Lit un export CSV du site CBOE (le tableau commence ligne 4)."""
    with open(filename) as handle:
        lines = handle.readlines()

    spot_price = float(lines[1].split("Last:")[1].split(",")[0])

    today = lines[2].split("Date: ")[1].split(",")
    month_day = today[0].split(" ")
    if len(month_day) == 2:  # format US : "September 15, 2024"
        year, month, day = int(today[1]), month_day[0], int(month_day[1])
    else:  # format EU : "15 September 2024"
        year, month, day = int(month_day[2]), month_day[1], int(month_day[0])
    quote_date = datetime.strptime(month, "%B").replace(day=day, year=year)

    df = pd.read_csv(filename, sep=",", header=None, skiprows=4)
    df.columns = COLUMNS
    df["ExpirationDate"] = (
        pd.to_datetime(df["ExpirationDate"], format="%a %b %d %Y") + timedelta(hours=16)
    )
    return _clean(df), spot_price, quote_date


def _clean(df):
    """Typage numérique + suppression des lignes inexploitables."""
    numeric = ["StrikePrice", "CallIV", "PutIV", "CallGamma", "PutGamma",
               "CallOpenInt", "PutOpenInt", "CallDelta", "PutDelta"]
    for col in numeric:
        df[col] = pd.to_numeric(df[col], errors="coerce")

    # Un strike sans OI ni des deux côtés n'apporte rien au calcul de GEX
    df[["CallOpenInt", "PutOpenInt"]] = df[["CallOpenInt", "PutOpenInt"]].fillna(0.0)
    df[["CallGamma", "PutGamma"]] = df[["CallGamma", "PutGamma"]].fillna(0.0)
    df[["CallIV", "PutIV"]] = df[["CallIV", "PutIV"]].fillna(0.0)
    df = df.dropna(subset=["StrikePrice", "ExpirationDate"])
    return df.sort_values(["ExpirationDate", "StrikePrice"]).reset_index(drop=True)


def to_cboe_csv(df, spot_price, quote_date, path):
    """Réécrit la chaîne au format CSV historique du CBOE (compat outils tiers)."""
    header = (
        f"Underlying,,,,,,,,,,,,,,,,,,,,,\n"
        f"Last: {spot_price},,,,,,,,,,,,,,,,,,,,\n"
        f"Date: {quote_date.strftime('%B %d')}, {quote_date.year},,,,,,,,,,,,,,,,,,,,\n"
    )
    body = io.StringIO()
    out = df.copy()
    out["ExpirationDate"] = out["ExpirationDate"].dt.strftime("%a %b %d %Y")
    out.to_csv(body, index=False)
    with open(path, "w", newline="") as handle:
        handle.write(header)
        handle.write(body.getvalue())
    return path


if __name__ == "__main__":
    import sys

    ticker = sys.argv[1] if len(sys.argv) > 1 else "SPCX"
    chain, spot, asof = fetch_chain(ticker)
    print(f"{ticker}: spot={spot} | {len(chain)} strikes | as of {asof}")
    print(f"expirations: {chain.ExpirationDate.nunique()} "
          f"({chain.ExpirationDate.min():%Y-%m-%d} -> {chain.ExpirationDate.max():%Y-%m-%d})")
    print(chain.head())
