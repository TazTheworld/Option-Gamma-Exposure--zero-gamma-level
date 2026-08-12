"""Historique des relevés : une ligne de CSV par exécution de main.py.

Sans ça, chaque analyse est un instantané et les séries n'existent nulle part —
il faut relire les sorties précédentes à la main pour voir une tendance.

    python main.py SPCX                 # enregistre automatiquement
    python main.py SPCX --no-history    # sauf si on ne veut pas

    python history.py                   # tous les tickers, dernier relevé de chacun
    python history.py SPCX              # l'historique de SPCX
    python history.py SPCX --last 5     # les 5 derniers

Le périmètre (dte_max) et la source de gamma sont enregistrés avec chaque ligne :
deux relevés du même jour qui n'en partagent pas ne sont pas comparables, et rien
d'autre ne permettrait de les distinguer. L'affichage les sépare donc en sections
plutôt que de les enchaîner dans une même série.
"""

import argparse
import os

import pandas as pd

DEFAUT = "history.csv"

COLONNES = ["timestamp", "ticker", "dte_max", "source_gamma", "time_convention",
            "spot", "total_gex", "zero_gamma", "call_wall", "put_wall",
            "call_wall_oi", "put_wall_oi", "charm", "vanna", "strikes", "expiries",
            # Contexte de séance du sous-jacent. Sans lui, l'historique ne permet
            # ni de rapporter le GEX au volume, ni de tester les murs sur le
            # parcours réel du jour, ni de comparer implicite et réalisé.
            "open", "high", "low", "close", "prev_close", "volume",
            "dollar_volume", "iv30", "gex_sur_volume"]

# Les colonnes de contexte portent le même nom dans le payload CBOE, sauf celle-ci.
MARCHE_ALIAS = {"prev_close": "prev_day_close"}

# Ce qui rend deux relevés comparables : même sous-jacent, même périmètre
# d'échéance, même source de gamma, même mesure du temps. Mélanger l'un des
# quatre compare des choses différentes — un dte_max de 7 et de 30 peuvent donner
# des GEX de signes opposés, deux sources de gamma plus de 10 % d'écart sur un
# indice, et deux conventions de temps déplacent le zero gamma de 12 points.
CLES = ["ticker", "dte_max", "source_gamma", "time_convention"]

# (colonne, libellé, largeur, format) — "prix" suit la précision du sous-jacent,
# "compact" abrège en M / Md pour que SPX et SPCX tiennent dans le même tableau
AFFICHAGE = [("timestamp", "date", 18, "texte"), ("spot", "spot", 11, "prix"),
             # en-têtes en ASCII : la console Windows (cp1252) ne sait pas encoder γ
             ("total_gex", "GEX", 11, "compact"), ("zero_gamma", "zero gam", 11, "prix"),
             ("call_wall", "call w.", 10, "prix"), ("put_wall", "put w.", 10, "prix"),
             ("call_wall_oi", "call OI", 10, "prix"), ("put_wall_oi", "put OI", 10, "prix"),
             ("charm", "charm/j", 11, "compact"), ("vanna", "vanna/pt", 11, "compact"),
             # Le ratio au volume est ce qui rend le GEX lisible : un montant nu
             # ne dit pas s'il pèse quelque chose face à ce qui s'échange.
             ("gex_sur_volume", "GEX/vol", 9, "pourcent"), ("iv30", "iv30", 8, "pourcent_nu")]


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


def _migrer(path):
    """Réécrit un historique antérieur à l'ajout d'une colonne.

    Sans ça, ajouter une ligne au nouveau format décale toutes les valeurs des
    lignes existantes d'une colonne, silencieusement.
    """
    existant = pd.read_csv(path)
    if list(existant.columns) == COLONNES:
        return
    for colonne in COLONNES:
        if colonne not in existant.columns:
            existant[colonne] = pd.NA
    existant[COLONNES].to_csv(path, index=False)


def record(path=DEFAUT, **valeurs):
    """Ajoute un relevé. Les clés inconnues sont ignorées, les absentes vides."""
    ligne = {c: valeurs.get(c) for c in COLONNES}
    if ligne["timestamp"] is None:
        ligne["timestamp"] = pd.Timestamp.now().strftime("%Y-%m-%d %H:%M")
    else:
        ligne["timestamp"] = pd.Timestamp(ligne["timestamp"]).strftime("%Y-%m-%d %H:%M")
    nouveau = not os.path.exists(path)
    if not nouveau:
        _migrer(path)
    pd.DataFrame([ligne])[COLONNES].to_csv(path, mode="a", header=nouveau, index=False)
    return path


def load(path=DEFAUT, ticker=None):
    if not os.path.exists(path):
        raise FileNotFoundError(
            f"Aucun historique dans {path}. Lance main.py au moins une fois "
            "(l'enregistrement est automatique, sauf --no-history)."
        )
    df = pd.read_csv(path)
    for colonne in COLONNES:          # historique écrit par une version antérieure
        if colonne not in df.columns:
            df[colonne] = pd.NA
    if ticker:
        df = df[df.ticker.str.upper() == ticker.upper().lstrip("_")]
    return df.sort_values("timestamp").reset_index(drop=True)


def _formate(valeur, decimales):
    if pd.isna(valeur):
        return "-"
    return f"{valeur:,.{decimales}f}"


def _prefixe(r):
    """Ce qui identifie un relevé : sous-jacent, périmètre, source, mesure du temps."""
    def texte(valeur):
        return "-" if pd.isna(valeur) else str(valeur)

    return (f"{str(r.ticker):<8}{texte(r.dte_max):>5}"
            f"{texte(r.source_gamma):>10}{texte(r.time_convention):>8}")


def _tableau(df, prec):
    def cellule(valeur, fmt, largeur):
        if fmt == "texte":
            return str(valeur)[:largeur]
        if fmt == "compact":
            return _compact(valeur)
        if fmt == "pourcent":       # une fraction (0,0087) -> 0,9 %
            return "-" if pd.isna(valeur) else f"{valeur * 100:,.1f}%"
        if fmt == "pourcent_nu":    # déjà en points de pourcentage (70,24)
            return "-" if pd.isna(valeur) else f"{valeur:,.1f}%"
        return _formate(valeur, prec)

    entete = (f"{'ticker':<8}{'dte':>5}{'gamma':>10}{'T':>8}"
              + "".join(f"{lib:>{w}}" for _, lib, w, _ in AFFICHAGE))
    print(entete)
    print("-" * len(entete))
    for _, r in df.iterrows():
        ligne = _prefixe(r)
        for col, _, w, fmt in AFFICHAGE:
            ligne += f"{cellule(r[col], fmt, w):>{w}}"
        print(ligne)


def show(path=DEFAUT, ticker=None, last=None):
    """Affiche l'historique, avec la variation entre le premier et le dernier relevé.

    Sans ticker, on montre le dernier relevé de chaque (sous-jacent, périmètre,
    source). Avec un ticker, une section par périmètre : enchaîner des relevés
    à 7 et à 30 jours dans une même série comparerait des grandeurs qui peuvent
    être de signes opposés le même jour.
    """
    df = load(path, ticker)
    if df.empty:
        print(f"aucun relevé pour {ticker}" if ticker else "historique vide")
        return df

    prec = 4 if df.spot.max() < 10 else 2      # les paires FX se lisent en pips

    if ticker is None:
        vue = df.groupby(CLES, as_index=False, dropna=False).last().sort_values(CLES)
        _tableau(vue, prec)
        return vue

    scopes = list(df.groupby(CLES[1:], dropna=False, sort=True))
    for cle, groupe in scopes:
        if len(scopes) > 1:
            dte, src, conv = cle
            print(f"\n=== périmètre dte_max={dte}, gamma={src}, T={conv} ===")
        _montrer_scope(groupe.tail(last) if last else groupe, prec)
    return df


def _montrer_scope(df, prec):
    _tableau(df, prec)
    if len(df) > 1:
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
            if fmt == "compact":
                fc = aff = _compact
            elif fmt in ("pourcent", "pourcent_nu"):
                # Une variation de pourcentage se lit en points, pas en dollars.
                facteur = 100 if fmt == "pourcent" else 1
                fc = lambda v, f=facteur: f"{v * f:+,.1f}pt"
                aff = lambda v, f=facteur: f"{v * f:,.1f}%"
            else:
                fc = lambda v: f"{v:+,.{prec}f}"
                aff = lambda v: _formate(v, prec)
            marque = "   changement de signe" if change_de_signe else ""
            print(f"  {lib:<10} {aff(a[col]):>12} -> {aff(b[col]):>12}   {fc(delta):>10}{pct}{marque}")
        signes = df.total_gex.dropna()
        if len(signes) > 1 and (signes > 0).any() and (signes < 0).any():
            print("\n  le régime a changé de signe sur la période "
                  "(gamma positif <-> négatif)")


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
