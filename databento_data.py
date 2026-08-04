"""Chaînes d'options CME via Databento (source licenciée, entièrement automatisée).

Alternative au téléchargement manuel de cme_data.py : Databento redistribue
légalement les données CME Globex (GLBX.MDP3), donc pas de scraping.

    export DATABENTO_API_KEY=db-xxxxxxxx
    python main.py 6E --databento                    # dernière séance close
    python main.py 6E --databento --date 2026-08-03

Deux requêtes par appel :
  - schema "definition"  -> strike, échéance, call/put de chaque contrat
  - schema "statistics"  -> open interest, prix de règlement, et si le CME le
    publie, la volatilité implicite

Le gamma n'étant jamais diffusé, il est calculé en Black-76 comme dans cme_data.

Attention : chaque requête est facturée au volume de données. Une journée
d'options 6E reste modeste, mais évite les boucles sur de longues périodes.
"""

import os
from datetime import date as _date, datetime, timedelta

import numpy as np
import pandas as pd

from cboe_data import COLUMNS, _clean
from cme_data import black76_gamma, implied_vol, infer_futures_price

DATASET = "GLBX.MDP3"


def _stat_types():
    """Codes stat_type, lus depuis l'énumération du paquet plutôt qu'en dur."""
    from databento_dbn import StatType
    return {
        "oi": int(StatType.OPEN_INTEREST),
        "settle": int(StatType.SETTLEMENT_PRICE),
        "volume": int(StatType.CLEARED_VOLUME),
        "vol": int(StatType.VOLATILITY),
    }


def _last_session(day=None):
    """Dernière séance close : la veille ouvrée (les stats sortent après clôture)."""
    day = pd.Timestamp(day).date() if day else _date.today()
    day -= timedelta(days=1)
    while day.weekday() >= 5:  # samedi/dimanche
        day -= timedelta(days=1)
    return day


def _descale(series, scale=1e9):
    """Databento renvoie parfois des prix en entiers fixes (1e-9). Normalise."""
    s = pd.to_numeric(series, errors="coerce")
    # Un strike EUR/USD vaut ~1, un strike ES ~5000 : au-delà de 1e6 c'est du fixe
    if s.notna().any() and s.abs().max() > 1e6:
        return s / scale
    return s


def fetch_chain(product="6E", day=None, api_key=None, dataset=DATASET, rate=0.0):
    """Télécharge la chaîne d'options d'un produit CME pour une séance.

    Renvoie (df, futures_price, quote_date) au format COLUMNS de cboe_data.
    """
    import databento as db

    api_key = api_key or os.environ.get("DATABENTO_API_KEY")
    if not api_key:
        raise ValueError(
            "Clé Databento absente : export DATABENTO_API_KEY=db-... "
            "(ou passe api_key=). Inscription sur https://databento.com"
        )

    session = _last_session(day) if day is None else pd.Timestamp(day).date()
    start = session.isoformat()
    end = (session + timedelta(days=1)).isoformat()
    client = db.Historical(api_key)

    def pull(schema, symbol):
        data = client.timeseries.get_range(
            dataset=dataset, schema=schema, symbols=[symbol],
            stype_in="parent", start=start, end=end,
        )
        return data.to_df()

    defs_df = pull("definition", f"{product}.OPT")
    stats_df = pull("statistics", f"{product}.OPT")
    if defs_df.empty:
        raise ValueError(
            f"Aucune définition d'option pour '{product}' le {session}. "
            "Vérifie le code produit (6E, ES, CL...) et que la séance est ouvrée."
        )

    futures_price = _front_future_settle(client, product, dataset, start, end)
    quote_date = datetime.combine(session, datetime.min.time())
    return build_chain(defs_df, stats_df, futures_price, quote_date, rate)


def _front_future_settle(client, product, dataset, start, end):
    """Règlement du future de première échéance ; None si indisponible."""
    try:
        defs = client.timeseries.get_range(
            dataset=dataset, schema="definition", symbols=[f"{product}.FUT"],
            stype_in="parent", start=start, end=end).to_df()
        stats = client.timeseries.get_range(
            dataset=dataset, schema="statistics", symbols=[f"{product}.FUT"],
            stype_in="parent", start=start, end=end).to_df()
    except Exception:
        return None  # on retombera sur la parité call-put

    if defs.empty or stats.empty:
        return None
    codes = _stat_types()
    settle = stats[stats.stat_type == codes["settle"]]
    if settle.empty:
        return None

    # Le future de première échéance porte le contrat le plus liquide
    exp = defs.drop_duplicates("instrument_id").set_index("instrument_id")["expiration"]
    settle = settle.join(exp, on="instrument_id").dropna(subset=["expiration"])
    if settle.empty:
        return None
    front = settle.sort_values("expiration").iloc[0]
    return float(_descale(pd.Series([front["price"]])).iloc[0])


def build_chain(defs_df, stats_df, futures_price=None, quote_date=None, rate=0.0):
    """Assemble définitions + statistiques en chaîne exploitable (fonction pure).

    Séparée de l'accès réseau pour être testable sans clé API.
    """
    codes = _stat_types()

    # --- définitions : un contrat par instrument_id ---
    defs = defs_df.copy()
    if "instrument_class" not in defs.columns:
        raise ValueError(f"Schéma 'definition' inattendu : {list(defs.columns)}")
    defs["cp"] = defs.instrument_class.astype(str).str.upper().str[0]
    defs = defs[defs.cp.isin(["C", "P"])]          # exclut futures et spreads
    defs = defs.drop_duplicates("instrument_id", keep="last")
    defs["StrikePrice"] = _descale(defs.strike_price)
    defs["ExpirationDate"] = pd.to_datetime(defs.expiration, errors="coerce", utc=True)
    defs["ExpirationDate"] = defs.ExpirationDate.dt.tz_localize(None)
    defs = defs.dropna(subset=["StrikePrice", "ExpirationDate"])
    defs = defs[["instrument_id", "cp", "StrikePrice", "ExpirationDate"]]
    if defs.empty:
        raise ValueError("Aucune option (call/put) dans les définitions reçues.")

    # --- statistiques : une valeur par (instrument, type de stat) ---
    stats = stats_df.copy()
    if "stat_type" not in stats.columns:
        raise ValueError(f"Schéma 'statistics' inattendu : {list(stats.columns)}")
    wanted = {codes["oi"]: "OpenInt", codes["settle"]: "Settle",
              codes["volume"]: "Vol", codes["vol"]: "IV"}
    stats = stats[stats.stat_type.isin(wanted)]
    # L'OI et le volume sont portés par `quantity`, les prix par `price`
    qty_types = {codes["oi"], codes["volume"]}
    stats["value"] = np.where(
        stats.stat_type.isin(qty_types),
        pd.to_numeric(stats.get("quantity"), errors="coerce"),
        _descale(stats.get("price")),
    )
    stats["field"] = stats.stat_type.map(wanted)
    stats = stats.dropna(subset=["value"])
    wide = (stats.groupby(["instrument_id", "field"])["value"].last()
                 .unstack("field") if not stats.empty else pd.DataFrame())

    df = defs.join(wide, on="instrument_id")
    for field in ("OpenInt", "Settle", "Vol", "IV"):
        if field not in df.columns:
            df[field] = np.nan

    # --- format large : une ligne par (échéance, strike) ---
    keys = ["ExpirationDate", "StrikePrice"]
    fields = ["Settle", "OpenInt", "Vol", "IV"]
    calls = df[df.cp == "C"].groupby(keys)[fields].last().add_prefix("Call")
    puts = df[df.cp == "P"].groupby(keys)[fields].last().add_prefix("Put")
    chain = calls.join(puts, how="outer").reset_index()

    quote_date = pd.Timestamp(quote_date or pd.Timestamp.utcnow().normalize())
    quote_date = quote_date.tz_localize(None) if quote_date.tz else quote_date
    quote_date = quote_date.to_pydatetime()

    if futures_price is None:
        futures_price = infer_futures_price(chain)
        if futures_price is None:
            raise ValueError(
                "Prix du future indéterminable : passe-le avec --futures-price."
            )
    futures_price = float(futures_price)

    T = ((chain.ExpirationDate - quote_date).dt.total_seconds()
         / (365.25 * 24 * 3600)).clip(lower=0)

    for side, opt in (("Call", "C"), ("Put", "P")):
        iv = pd.to_numeric(chain[f"{side}IV"], errors="coerce")
        if iv.notna().any() and iv.max(skipna=True) > 3:   # publiée en pourcentage
            iv = iv / 100.0
        missing = iv.isna() & chain[f"{side}Settle"].notna()
        if missing.any():
            iv.loc[missing] = [
                implied_vol(p, futures_price, k, t, rate, opt)
                for p, k, t in zip(chain.loc[missing, f"{side}Settle"],
                                   chain.loc[missing, "StrikePrice"], T[missing])
            ]
        chain[f"{side}IV"] = iv
        chain[f"{side}Gamma"] = black76_gamma(futures_price, chain.StrikePrice, iv, T, rate)

    chain["Calls"] = ""
    chain["Puts"] = ""
    chain = chain.reindex(columns=COLUMNS)
    return _clean(chain), futures_price, quote_date


if __name__ == "__main__":
    import sys

    produit = sys.argv[1] if len(sys.argv) > 1 else "6E"
    jour = sys.argv[2] if len(sys.argv) > 2 else None
    chaine, prix, date_val = fetch_chain(produit, jour)
    print(f"{produit} : future {prix:.5f} | {len(chaine)} strikes | {date_val:%Y-%m-%d}")
    print(f"OI total : calls {chaine.CallOpenInt.sum():,.0f} "
          f"puts {chaine.PutOpenInt.sum():,.0f}")
    print(chaine.head())
