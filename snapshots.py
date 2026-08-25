"""Archivage des chaînes d'options brutes, une par exécution.

history.csv ne garde que les agrégats dérivés (GEX, zero gamma, murs). C'est
suffisant pour tracer une série, mais ça ferme trois portes :

  - impossible de rejouer une séance passée avec un autre --dte-max ou une autre
    source de gamma, donc impossible de corriger une erreur de méthode
    rétroactivement — il faudrait attendre que l'historique se reconstitue ;
  - impossible de mesurer autre chose que ce qui a été mesuré ce jour-là ;
  - la validation est réduite à la colonne `spot`, soit un point par exécution.

Une chaîne SPX complète pèse quelques centaines de Ko en parquet. On garde donc
tout le brut, et on rejoue à volonté :

    python main.py _SPX                                  # archive au passage
    python main.py _SPX --replay snapshots/SPX/2026-08-12_1436.parquet --dte-max 7

Le spot et la date de valorisation voyagent avec la chaîne, en colonnes
constantes : parquet les compresse à presque rien, et aucun fichier annexe ne
peut se désolidariser des données.
"""

import glob
import os

import pandas as pd

DOSSIER = "snapshots"
META_SPOT = "_spot"
META_DATE = "_quote_date"
PREFIXE_MARCHE = "_marche_"
NOM_COURANT = "courant"


def _sans_parquet():
    """parquet demande pyarrow, qui n'est pas dans les dépendances de base."""
    try:
        import pyarrow  # noqa: F401
        return False
    except ImportError:
        return True


def chemin(ticker, quote_date, dossier=DOSSIER):
    """snapshots/SPX/2026-08-12_1436.parquet"""
    horodatage = pd.Timestamp(quote_date).strftime("%Y-%m-%d_%H%M")
    extension = "csv.gz" if _sans_parquet() else "parquet"
    return os.path.join(dossier, str(ticker).upper(), f"{horodatage}.{extension}")


def courant(ticker, dossier=DOSSIER):
    """snapshots/NQ/courant.parquet — le relevé vivant, réécrit en place.

    chemin() horodate à la minute. Un collecteur qui réécrit toutes les quinze
    secondes y créerait un fichier neuf par minute, soit près de mille cinq cents
    par jour et par sous-jacent : une archive illisible, et un lecteur incapable
    de savoir lequel est le dernier sans lister le dossier à chaque fois.

    Le courant a donc un chemin fixe, et les archives horodatées gardent
    chemin(). Même format dans les deux cas : sauver() et charger() servent les
    deux sans changement, et main.py --replay ouvre l'un comme l'autre.
    """
    extension = "csv.gz" if _sans_parquet() else "parquet"
    return os.path.join(dossier, str(ticker).upper(), f"{NOM_COURANT}.{extension}")


def sauver(df, ticker, spot, quote_date, dossier=DOSSIER, marche=None):
    """Écrit la chaîne brute et renvoie le chemin. Écrase un relevé du même horodatage.

    Le contexte de séance (OHLCV, iv30) voyage avec la chaîne : sans lui, une
    archive rejouée plus tard ne permet plus de rapporter le GEX au volume ni de
    tester les murs sur le parcours réel du jour.
    """
    cible = chemin(ticker, quote_date, dossier)
    os.makedirs(os.path.dirname(cible), exist_ok=True)
    out = df.copy()
    out[META_SPOT] = float(spot)
    out[META_DATE] = pd.Timestamp(quote_date)
    for champ, valeur in (marche or {}).items():
        out[f"{PREFIXE_MARCHE}{champ}"] = valeur
    if cible.endswith(".parquet"):
        out.to_parquet(cible, index=False)
    else:
        out.to_csv(cible, index=False, compression="gzip")
    return cible


def charger(source):
    """Relit une chaîne archivée -> (df, spot, quote_date, marche).

    `marche` est vide pour les archives écrites avant qu'on garde le contexte de
    séance : le relevé reste exploitable, seuls les ratios au volume manquent.
    """
    if not os.path.exists(source):
        raise FileNotFoundError(f"Aucun relevé archivé à {source}.")
    df = pd.read_parquet(source) if source.endswith(".parquet") else pd.read_csv(source)

    manquantes = {META_SPOT, META_DATE, "ExpirationDate"} - set(df.columns)
    if manquantes:
        raise ValueError(f"{source} n'est pas un relevé archivé "
                         f"(colonnes absentes : {sorted(manquantes)}).")
    spot = float(df[META_SPOT].iloc[0])
    quote_date = pd.Timestamp(df[META_DATE].iloc[0]).to_pydatetime()

    colonnes_marche = [c for c in df.columns if c.startswith(PREFIXE_MARCHE)]
    marche = {}
    for colonne in colonnes_marche:
        valeur = df[colonne].iloc[0]
        marche[colonne[len(PREFIXE_MARCHE):]] = None if pd.isna(valeur) else float(valeur)

    df = df.drop(columns=[META_SPOT, META_DATE, *colonnes_marche])
    df["ExpirationDate"] = pd.to_datetime(df["ExpirationDate"])
    return df, spot, quote_date, marche


def lister(ticker=None, dossier=DOSSIER):
    """Chemins archivés, du plus ancien au plus récent.

    Le relevé courant est exclu : il porte le même format que les archives, mais
    pas le même rôle. Le laisser entrer le ferait remonter dans dernier(), donc
    dans les relectures et dans validate.py — où un fichier qui change sous les
    pieds n'a rien à faire.
    """
    motif = os.path.join(dossier, str(ticker).upper() if ticker else "*", "*.*")
    return sorted(f for f in glob.glob(motif)
                  if f.endswith((".parquet", ".csv.gz"))
                  and not os.path.basename(f).startswith(NOM_COURANT + "."))


def dernier(ticker, dossier=DOSSIER):
    """Relevé archivé le plus récent d'un sous-jacent, ou None."""
    trouves = lister(ticker, dossier)
    return trouves[-1] if trouves else None


def main():
    import argparse

    p = argparse.ArgumentParser(description="Relevés de chaînes archivés")
    p.add_argument("ticker", nargs="?", help="sous-jacent ; omis, montre tout")
    p.add_argument("--dir", default=DOSSIER, help=f"dossier d'archives (défaut : {DOSSIER})")
    args = p.parse_args()

    trouves = lister(args.ticker, args.dir)
    if not trouves:
        print(f"aucun relevé archivé dans {args.dir}"
              + (f" pour {args.ticker}" if args.ticker else ""))
        return
    for f in trouves:
        taille = os.path.getsize(f) / 1024
        print(f"{f}  ({taille:,.0f} Ko)")
    print(f"\n{len(trouves)} relevés — rejouable avec : python main.py <TICKER> --replay <fichier>")


if __name__ == "__main__":
    main()
