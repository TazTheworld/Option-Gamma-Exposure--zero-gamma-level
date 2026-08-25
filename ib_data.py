"""Collecteur Interactive Brokers pour les options sur futures NQ.

IB ne sert pas une chaîne : il sert des contrats un par un, avec un plafond de
cent lignes de données simultanées. Tout ce module découle de cette contrainte.

Ce fichier ne contient que les fonctions pures — celles qui décident, assemblent
et fusionnent, et qui se vérifient sans réseau. C'est le patron de
databento_data.py, pour la même raison qui y est écrite : ce qui produit un
chiffre doit être testable hors ligne.

    perimetre()      quels contrats demander, avant de les demander
    build_chain()    définitions + valeurs -> chaîne au format du projet
    selection_vif()  les contrats à garder souscrits en permanence
    fusionner()      open interest du socle + IV du vif -> entrée d'analyser()

La connexion, l'énumération et la souscription vivent ailleurs : elles demandent
IB Gateway, donc elles ne se testent pas ici. Rien dans ce fichier n'importe
ib_async — `python main.py TSLA` ne doit pas payer une dépendance de courtier.

Conception : docs/superpowers/specs/2026-08-25-collecteur-ib-nq-design.md
"""

import numpy as np
import pandas as pd

# La forme unique d'un contrat dans ce module. La couche réseau construira ses
# objets ib_async.Contract à partir de ces trois colonnes, et rien d'autre :
# perimetre() et selection_vif() rendent toutes deux cette forme, pour qu'il n'y
# ait jamais deux façons de désigner un contrat.
CONTRAT = ["ExpirationDate", "StrikePrice", "right"]


def _quote_date(valeur=None):
    """Date de valorisation normalisée, sans fuseau."""
    ts = pd.Timestamp(valeur) if valeur is not None else pd.Timestamp.now("UTC")
    if ts.tz is not None:
        ts = ts.tz_localize(None)
    return ts.normalize()


def perimetre(strikes, echeances, prix, plage=0.2, dte_max=30, dte_min=0,
              quote_date=None):
    """Les contrats à demander, un par (échéance, strike, sens).

    C'est l'inversion que le projet n'avait jamais eu à faire. Le CBOE sert toute
    la chaîne et analysis.filtre_echeances() élague ensuite ; IB oblige à élaguer
    AVANT de demander, chaque contrat coûtant une des cent lignes disponibles.
    `plage` et `dte_max` cessent donc d'être des réglages d'affichage pour devenir
    le périmètre d'acquisition.

    Le périmètre demandé n'est pas le périmètre obtenu, et ce n'est pas une
    anomalie : le CME ne liste que vingt-cinq strikes de part et d'autre du
    règlement sur les échéances hebdomadaires, soit environ ±2,5 %. Ce qui n'est
    pas listé n'apparaît simplement pas dans `strikes`, et une trame vide est une
    réponse, pas une panne.

    L'ordre du résultat est celui dans lequel les lots partiront. Il est donc
    trié, et deux appels sur les mêmes entrées rendent la même liste : sans ça,
    chaque recyclage de lignes re-souscrirait les mêmes contrats dans le désordre.

    `dte_max=None` garde toutes les échéances (le 'all' de main.py).
    """
    quote_date = _quote_date(quote_date)

    ks = pd.to_numeric(pd.Series(list(strikes), dtype="object"), errors="coerce").dropna()
    bas, haut = float(prix) * (1 - plage), float(prix) * (1 + plage)
    # Les bornes sont calculées en flottant, donc fausses de quelques femtomètres :
    # 25 000 x 1,025 vaut 25 624,999999999996, ce qui EXCLUT le strike 25 625
    # pourtant demandé. Sans marge, on perdrait le strike le plus éloigné — celui
    # qui borne le profil de gamma — et de façon imprévisible, puisque l'erreur
    # dépend du prix du future et change donc d'une séance à l'autre.
    marge = abs(float(prix)) * 1e-9
    ks = sorted({float(k) for k in ks if bas - marge <= k <= haut + marge})

    dates = pd.to_datetime(pd.Series(list(echeances), dtype="object"),
                           errors="coerce").dropna()
    gardees = []
    for d in sorted({pd.Timestamp(x).normalize() for x in dates}):
        jours = (d - quote_date).days
        if jours < dte_min:
            continue
        if dte_max is not None and jours > dte_max:
            continue
        gardees.append(d)

    lignes = [{"ExpirationDate": d, "StrikePrice": k, "right": r}
              for d in gardees for k in ks for r in ("C", "P")]
    df = pd.DataFrame(lignes, columns=CONTRAT)
    df["ExpirationDate"] = pd.to_datetime(df["ExpirationDate"])
    df["StrikePrice"] = pd.to_numeric(df["StrikePrice"], errors="coerce")
    return df


if __name__ == "__main__":
    # Sans réseau : ce qu'un périmètre coûterait en contrats, donc en temps de
    # balayage. À lire avec la mise en garde ci-dessous — le produit cartésien
    # n'existe pas sur le marché.
    import sys

    prix = float(sys.argv[1]) if len(sys.argv) > 1 else 25_000.0
    plage = float(sys.argv[2]) if len(sys.argv) > 2 else 0.2
    strikes = np.arange(prix * 0.7, prix * 1.3, 25.0)
    echeances = pd.date_range(pd.Timestamp.today().normalize(), periods=30, freq="D")

    p = perimetre(strikes, echeances, prix, plage=plage)
    lots = -(-len(p) // 90)
    print(f"NQ à {prix:,.0f}, plage ±{plage:.1%} | {p.StrikePrice.nunique()} strikes "
          f"x {p.ExpirationDate.nunique()} échéances x 2 = {len(p):,} contrats")
    print(f"{lots} lots de 90 — compter deux à quatre secondes par lot, "
          f"soit {lots * 3 / 60:.0f} min")
    print()
    print("ATTENTION : c'est un MAJORANT, pas une prévision. reqSecDefOptParams")
    print("rend l'union des strikes et l'union des échéances, pas les couples")
    print("réellement cotés — et le CME ne liste que 25 strikes de part et")
    print("d'autre du règlement sur les échéances hebdomadaires, soit ±2,5 %.")
    print("La plupart des couples comptés ici n'existent donc pas.")
