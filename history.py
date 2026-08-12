"""Historique des relevés : une ligne de CSV par exécution de main.py.

Sans ça, chaque analyse est un instantané et les séries n'existent nulle part —
il faut relire les sorties précédentes à la main pour voir une tendance.

    python main.py SPCX                 # enregistre automatiquement
    python main.py SPCX --no-history    # sauf si on ne veut pas

    python history.py                   # tous les tickers, dernier relevé de chacun
    python history.py SPCX              # l'historique de SPCX
    python history.py SPCX --last 5     # les 5 derniers

Le périmètre (dte_max) est enregistré avec chaque ligne : deux relevés du même
jour sur des horizons différents ne sont pas comparables, et rien d'autre ne
permettrait de les distinguer.
"""

import argparse
import os

import pandas as pd

DEFAUT = "history.csv"

COLONNES = ["timestamp", "ticker", "dte_max", "spot", "total_gex", "zero_gamma",
            "call_wall", "put_wall", "call_wall_oi", "put_wall_oi",
            "charm", "vanna", "strikes", "expiries"]

# (colonne, libellé, largeur, format) — "prix" suit la précision du sous-jacent,
# "compact" abrège en M / Md pour que SPX et SPCX tiennent dans le même tableau
AFFICHAGE = [("timestamp", "date", 18, "texte"), ("spot", "spot", 11, "prix"),
             # en-têtes en ASCII : la console Windows (cp1252) ne sait pas encoder γ
             ("total_gex", "GEX", 11, "compact"), ("zero_gamma", "zero gam", 11, "prix"),
             ("call_wall", "call w.", 10, "prix"), ("put_wall", "put w.", 10, "prix"),
             ("call_wall_oi", "call OI", 10, "prix"), ("put_wall_oi", "put OI", 10, "prix"),
             ("charm", "charm/j", 11, "compact"), ("vanna", "vanna/pt", 11, "compact")]


def _compact(v):
    """58 107 413 -> +58.1M ; 49 253 080 615 -> +49.25Md. Sinon le tableau déborde."""
    if pd.isna(v):
        return "-"
    a = abs(v)
    if a >= 1e9:
        return f"{v / 1e9:+,.2f}Md"
    if a >= 1e6:
        return f"{v / 1e6:+,.1f}M"
    if a >= 1e3:
        return f"{v / 1e3:+,.1f}k"
    return f"{v:+,.0f}"


def record(path=DEFAUT, **valeurs):
    """Ajoute un relevé. Les clés inconnues sont ignorées, les absentes vides."""
    ligne = {c: valeurs.get(c) for c in COLONNES}
    if ligne["timestamp"] is None:
        ligne["timestamp"] = pd.Timestamp.now().strftime("%Y-%m-%d %H:%M")
    else:
        ligne["timestamp"] = pd.Timestamp(ligne["timestamp"]).strftime("%Y-%m-%d %H:%M")
    nouveau = not os.path.exists(path)
    pd.DataFrame([ligne])[COLONNES].to_csv(path, mode="a", header=nouveau, index=False)
    return path


def load(path=DEFAUT, ticker=None):
    if not os.path.exists(path):
        raise FileNotFoundError(
            f"Aucun historique dans {path}. Lance main.py au moins une fois "
            "(l'enregistrement est automatique, sauf --no-history)."
        )
    df = pd.read_csv(path)
    if ticker:
        df = df[df.ticker.str.upper() == ticker.upper().lstrip("_")]
    return df.sort_values("timestamp").reset_index(drop=True)


def _formate(valeur, decimales):
    if pd.isna(valeur):
        return "-"
    return f"{valeur:,.{decimales}f}"


def show(path=DEFAUT, ticker=None, last=None):
    """Affiche l'historique, avec la variation entre le premier et le dernier relevé."""
    df = load(path, ticker)
    if df.empty:
        print(f"aucun relevé pour {ticker}" if ticker else "historique vide")
        return df

    if ticker is None:
        # vue d'ensemble : le dernier relevé de chaque ticker
        df = df.groupby("ticker", as_index=False).last().sort_values("ticker")
    elif last:
        df = df.tail(last)

    prec = 4 if df.spot.max() < 10 else 2      # les paires FX se lisent en pips

    def cellule(valeur, fmt, largeur):
        if fmt == "texte":
            return str(valeur)[:largeur]
        if fmt == "compact":
            return _compact(valeur)
        return _formate(valeur, prec)

    entete = f"{'ticker':<8}" + "".join(f"{lib:>{w}}" for _, lib, w, _ in AFFICHAGE)
    print(entete)
    print("-" * len(entete))
    for _, r in df.iterrows():
        ligne = f"{r.ticker:<8}"
        for col, _, w, fmt in AFFICHAGE:
            ligne += f"{cellule(r[col], fmt, w):>{w}}"
        print(ligne)

    if ticker and len(df) > 1:
        a, b = df.iloc[0], df.iloc[-1]
        print(f"\nvariation sur {len(df)} relevés, du {str(a.timestamp)[:10]} au {str(b.timestamp)[:10]} :")
        for col, lib, _, fmt in AFFICHAGE[1:]:
            if pd.isna(a[col]) or pd.isna(b[col]):
                continue
            delta = b[col] - a[col]
            # Un pourcentage n'a pas de sens quand la grandeur change de signe
            # (passer de -58 M$ à +60 M$ n'est pas « -203 % »).
            change_de_signe = a[col] * b[col] < 0
            pct = "" if (not a[col] or change_de_signe) else f"  ({delta / a[col] * 100:+.1f}%)"
            fc = _compact if fmt == "compact" else (lambda v: f"{v:+,.{prec}f}")
            aff = _compact if fmt == "compact" else (lambda v: _formate(v, prec))
            marque = "   changement de signe" if change_de_signe else ""
            print(f"  {lib:<10} {aff(a[col]):>12} -> {aff(b[col]):>12}   {fc(delta):>10}{pct}{marque}")
        signes = df.total_gex.dropna()
        if len(signes) > 1 and (signes > 0).any() and (signes < 0).any():
            print("\n  le régime a changé de signe sur la période "
                  "(gamma positif <-> négatif)")
    return df


def main():
    p = argparse.ArgumentParser(description="Historique des relevés de gamma exposure")
    p.add_argument("ticker", nargs="?", help="sous-jacent ; omis, affiche le dernier de chacun")
    p.add_argument("--file", default=DEFAUT, help=f"fichier d'historique (défaut : {DEFAUT})")
    p.add_argument("--last", type=int, help="ne montrer que les N derniers relevés")
    args = p.parse_args()
    show(args.file, args.ticker, args.last)


if __name__ == "__main__":
    try:
        main()
    except (FileNotFoundError, ValueError) as err:
        raise SystemExit(f"Erreur : {err}")
