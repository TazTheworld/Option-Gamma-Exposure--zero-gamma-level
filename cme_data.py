"""Lecture des exports de règlement d'options CME (6E Euro FX, ES, ...).

Le CME interdit l'accès automatisé à son site (Data Terms of Use) : on part donc
d'un fichier téléchargé à la main depuis l'Option Settlement Tool
(https://www.cmegroup.com/tools-information/quikstrike/option-settlement.html),
et ce module le convertit au format attendu par main.py.

Deux différences avec le CBOE :
  - le CME ne publie pas le gamma : on le calcule en Black-76 à partir de l'IV
    (et si l'IV manque, on l'inverse depuis le prix de règlement) ;
  - le sous-jacent est le future, pas le spot. Le prix du future peut être donné
    explicitement, lu dans l'en-tête, ou déduit par parité call-put.

Le format exact des colonnes CME n'étant pas documenté publiquement, la détection
est tolérante (alias, insensible à la casse). Utiliser inspect() en cas d'échec.
"""

import re
from datetime import timedelta

import numpy as np
import pandas as pd
from scipy.optimize import brentq
from scipy.stats import norm

from cboe_data import COLUMNS, _clean

# Multiplicateurs des principaux contrats CME
CONTRACT_SIZES = {"6E": 125_000, "6B": 62_500, "6J": 12_500_000, "6A": 100_000,
                  "ES": 50, "NQ": 20, "CL": 1_000, "GC": 100}

# Alias de colonnes, du plus spécifique au plus générique
ALIASES = {
    "strike": ["strike price", "strike", "exercise price"],
    "type": ["put/call", "call/put", "type", "cp", "option type"],
    "settle": ["settlement price", "settlement", "settle", "prior settle", "last price", "last"],
    "oi": ["prior day oi", "open interest", "prior int", "at close", "oi"],
    "iv": ["implied volatility", "implied vol", "volatility", "impl vol", "iv"],
    "volume": ["est. volume", "estimated volume", "volume", "est vol", "vol"],
    "expiry": ["expiration date", "expiration", "expiry", "maturity", "contract month", "month"],
}


def _norm(name):
    """canonicalise un nom de colonne : minuscules, sans ponctuation superflue."""
    return re.sub(r"[^a-z0-9 /]", " ", str(name).lower()).strip()


def _find(columns, key, prefix=None):
    """Retrouve la colonne correspondant à `key`, éventuellement préfixée call/put."""
    normed = {c: _norm(c) for c in columns}
    for alias in ALIASES[key]:
        for col, n in normed.items():
            if prefix:
                if n.startswith(prefix) and alias in n:
                    return col
            elif alias == n:
                return col
    if prefix:
        return None
    # 2e passe : correspondance partielle (ex. "Settle (USD)")
    for alias in ALIASES[key]:
        for col, n in normed.items():
            if alias in n:
                return col
    return None


def inspect(path):
    """Affiche les colonnes détectées — à lancer si le parsing échoue."""
    raw = _read_any(path)
    print(f"colonnes du fichier ({len(raw.columns)}) :")
    for c in raw.columns:
        print(f"  - {c!r}")
    print("\ndétection :")
    for key in ALIASES:
        print(f"  {key:<8} -> {_find(raw.columns, key)!r}")
    print("\nformat large (calls/puts séparés) :")
    for key in ["settle", "oi", "iv"]:
        print(f"  call {key:<7} -> {_find(raw.columns, key, 'call')!r}"
              f"   put {key:<7} -> {_find(raw.columns, key, 'put')!r}")
    print(raw.head())
    return raw


def _read_any(path):
    """Lit un CSV ou un XLS(X), en sautant les lignes d'en-tête non tabulaires."""
    if str(path).lower().endswith((".xls", ".xlsx")):
        return pd.read_excel(path)

    # Le CME préfixe parfois le tableau de lignes de titre : on cherche la ligne
    # d'en-tête, définie comme la première contenant "strike".
    with open(path, encoding="utf-8-sig", errors="replace") as handle:
        lines = handle.readlines()
    header = next((i for i, l in enumerate(lines) if "strike" in l.lower()), 0)
    return pd.read_csv(path, skiprows=header)


# ---=== Black-76 ===---

def _d1(F, K, vol, T):
    return (np.log(F / K) + 0.5 * vol ** 2 * T) / (vol * np.sqrt(T))


def black76_price(F, K, vol, T, r, opt_type):
    if T <= 0 or vol <= 0:
        return max(F - K, 0.0) if opt_type == "C" else max(K - F, 0.0)
    d1 = _d1(F, K, vol, T)
    d2 = d1 - vol * np.sqrt(T)
    if opt_type == "C":
        return np.exp(-r * T) * (F * norm.cdf(d1) - K * norm.cdf(d2))
    return np.exp(-r * T) * (K * norm.cdf(-d2) - F * norm.cdf(-d1))


def black76_gamma(F, K, vol, T, r=0.0):
    """Gamma d'une option sur future. À r = 0, identique au gamma Black-Scholes."""
    F, K, vol, T = (np.asarray(x, dtype=float) for x in (F, K, vol, T))
    valid = (T > 0) & (vol > 0) & (K > 0) & (F > 0)
    vol_s, T_s, K_s, F_s = (np.where(valid, x, 1.0) for x in (vol, T, K, F))
    d1 = _d1(F_s, K_s, vol_s, T_s)
    gamma = np.exp(-r * T_s) * norm.pdf(d1) / (F_s * vol_s * np.sqrt(T_s))
    return np.where(valid, gamma, 0.0)


def implied_vol(price, F, K, T, r, opt_type):
    """Inverse la vol depuis le prix de règlement (utilisé si l'IV manque)."""
    intrinsic = max(F - K, 0.0) if opt_type == "C" else max(K - F, 0.0)
    if not np.isfinite(price) or price <= intrinsic * np.exp(-r * T) or T <= 0:
        return np.nan
    try:
        return brentq(lambda v: black76_price(F, K, v, T, r, opt_type) - price,
                      1e-6, 5.0, xtol=1e-8, maxiter=200)
    except (ValueError, RuntimeError):
        return np.nan


def infer_futures_price(df):
    """Déduit le prix du future par parité call-put au strike le plus ATM.

    Black-76 à r = 0 : C - P = F - K, donc F = K + (C - P) là où |C - P| est
    minimal (le strike le plus proche de la monnaie).
    """
    ok = df.dropna(subset=["CallSettle", "PutSettle", "StrikePrice"])
    if ok.empty:
        return None
    diff = (ok.CallSettle - ok.PutSettle).abs()
    row = ok.loc[diff.idxmin()]
    return float(row.StrikePrice + row.CallSettle - row.PutSettle)


# ---=== Chargement ===---

def load_settlement(path, futures_price=None, expiry=None, quote_date=None,
                    rate=0.0, product="6E"):
    """Lit un export de règlement CME et renvoie (df, futures_price, quote_date).

    df suit le format COLUMNS de cboe_data, donc main.py le consomme tel quel.
    """
    raw = _read_any(path)
    raw.columns = [str(c).strip() for c in raw.columns]

    col_strike = _find(raw.columns, "strike")
    if col_strike is None:
        raise ValueError(
            f"Colonne 'strike' introuvable dans {path}. "
            f"Colonnes vues : {list(raw.columns)}. "
            "Lance cme_data.inspect(chemin) et envoie la sortie."
        )

    wide = _find(raw.columns, "settle", "call") is not None
    df = _parse_wide(raw, col_strike) if wide else _parse_long(raw, col_strike)
    df = df.dropna(subset=["StrikePrice"])
    if df.empty:
        raise ValueError(f"Aucune ligne exploitable dans {path}.")

    # --- échéance ---
    if expiry is not None:
        df["ExpirationDate"] = pd.to_datetime(expiry) + timedelta(hours=16)
    else:
        col_exp = _find(raw.columns, "expiry")
        if col_exp is None:
            raise ValueError(
                "Échéance absente du fichier : passe-la avec --expiry AAAA-MM-JJ."
            )
        df["ExpirationDate"] = pd.to_datetime(
            raw.loc[df.index, col_exp], errors="coerce"
        ) + timedelta(hours=16)
        df = df.dropna(subset=["ExpirationDate"])

    quote_date = pd.to_datetime(quote_date) if quote_date else pd.Timestamp.utcnow().normalize()
    quote_date = quote_date.to_pydatetime().replace(tzinfo=None)

    # --- prix du future ---
    if futures_price is None:
        futures_price = infer_futures_price(df)
        if futures_price is None:
            raise ValueError(
                "Prix du future indéterminable (parité call-put impossible) : "
                "passe-le avec --futures-price."
            )
    futures_price = float(futures_price)

    # --- maturité en années ---
    T = ((df.ExpirationDate - quote_date).dt.total_seconds() / (365.25 * 24 * 3600)).clip(lower=0)

    # --- IV : celle du fichier, sinon inversée depuis le prix de règlement ---
    for side, opt in (("Call", "C"), ("Put", "P")):
        iv = pd.to_numeric(df.get(f"{side}IV"), errors="coerce")
        if iv is None or iv.isna().all():
            iv = pd.Series(np.nan, index=df.index)
        # Le CME exprime parfois l'IV en pourcentage
        if iv.notna().any() and iv.max(skipna=True) > 3:
            iv = iv / 100.0
        missing = iv.isna() & df[f"{side}Settle"].notna()
        if missing.any():
            iv.loc[missing] = [
                implied_vol(p, futures_price, k, t, rate, opt)
                for p, k, t in zip(df.loc[missing, f"{side}Settle"],
                                   df.loc[missing, "StrikePrice"], T[missing])
            ]
        df[f"{side}IV"] = iv
        df[f"{side}Gamma"] = black76_gamma(futures_price, df.StrikePrice, iv, T, rate)

    df["Calls"] = ""
    df["Puts"] = ""
    df = df.reindex(columns=COLUMNS)
    return _clean(df), futures_price, quote_date


def _parse_wide(raw, col_strike):
    """Format CME natif : calls à gauche du strike, puts à droite."""
    out = pd.DataFrame(index=raw.index)
    out["StrikePrice"] = pd.to_numeric(raw[col_strike], errors="coerce")
    for side, prefix in (("Call", "call"), ("Put", "put")):
        for field, key in (("Settle", "settle"), ("OpenInt", "oi"),
                           ("IV", "iv"), ("Vol", "volume")):
            col = _find(raw.columns, key, prefix)
            out[f"{side}{field}"] = pd.to_numeric(raw[col], errors="coerce") if col else np.nan
    return out


def _parse_long(raw, col_strike):
    """Format une ligne par contrat, avec une colonne Call/Put."""
    col_type = _find(raw.columns, "type")
    if col_type is None:
        raise ValueError(
            "Ni colonnes call/put séparées, ni colonne 'Type' : format non reconnu. "
            "Lance cme_data.inspect(chemin) et envoie la sortie."
        )
    raw = raw.copy()
    raw["_k"] = pd.to_numeric(raw[col_strike], errors="coerce")
    raw["_cp"] = raw[col_type].astype(str).str.strip().str.upper().str[0]

    fields = {"Settle": "settle", "OpenInt": "oi", "IV": "iv", "Vol": "volume"}
    cols = {f: _find(raw.columns, k) for f, k in fields.items()}

    frames = []
    for cp, side in (("C", "Call"), ("P", "Put")):
        part = raw[raw._cp == cp]
        block = pd.DataFrame({"StrikePrice": part._k})
        for field, col in cols.items():
            block[f"{side}{field}"] = pd.to_numeric(part[col], errors="coerce") if col else np.nan
        # on conserve l'échéance pour l'alignement, si présente
        frames.append(block.groupby("StrikePrice").first())

    out = frames[0].join(frames[1], how="outer").reset_index()
    return out


if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print(__doc__)
        print("usage: python cme_data.py <fichier.csv|xlsx> [--inspect]")
        raise SystemExit(1)
    if "--inspect" in sys.argv:
        inspect(sys.argv[1])
    else:
        chain, price, date = load_settlement(sys.argv[1])
        print(f"future = {price:.5f} | {len(chain)} strikes | {date:%Y-%m-%d}")
        print(chain.head())
