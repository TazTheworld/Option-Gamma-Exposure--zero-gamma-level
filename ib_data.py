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

from cboe_data import COLONNES_GRECS, COLUMNS, _clean
from cme_data import black76_gamma, implied_vol, infer_futures_price

# La forme unique d'un contrat dans ce module. La couche réseau construira ses
# objets ib_async.Contract à partir de ces trois colonnes, et rien d'autre :
# perimetre() et selection_vif() rendent toutes deux cette forme, pour qu'il n'y
# ait jamais deux façons de désigner un contrat.
CONTRAT = ["ExpirationDate", "StrikePrice", "right"]

# Ce que la couche réseau collecte par contrat. Les colonnes absentes valent NaN
# plutôt que de manquer : un contrat illiquide qui ne répond jamais ne doit pas
# faire échouer l'assemblage des trois mille autres.
CHAMPS_TICK = ["OpenInt", "IV", "Gamma", "Delta", "Vega", "Theta", "Settle"]


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


def build_chain(defs, ticks, futures_price=None, quote_date=None, rate=0.0):
    """Définitions + valeurs reçues -> chaîne au format pivot du projet.

    Jumelle de databento_data.build_chain(), et pour les mêmes raisons : des
    définitions d'un côté, des valeurs de l'autre, une fonction pure au milieu.
    Séparée de l'accès réseau pour être testable sans IB Gateway. Deux
    assembleurs qui divergeraient sur la normalisation de l'IV donneraient deux
    GEX différents pour la même chaîne — d'où le calque plutôt que l'invention.

    `defs`  : conId, StrikePrice, ExpirationDate, right
    `ticks` : conId + ce que la souscription a rendu (CHAMPS_TICK)

    Renvoie (df, futures_price, quote_date) au format COLUMNS.
    """
    manquantes = {"conId", "StrikePrice", "ExpirationDate", "right"} - set(defs.columns)
    if manquantes:
        raise ValueError(
            f"Définitions IB inattendues, colonnes absentes : {sorted(manquantes)}. "
            f"Reçu : {list(defs.columns)}"
        )

    d = defs.copy()
    d["right"] = d["right"].astype(str).str.upper().str[0]
    d = d[d.right.isin(["C", "P"])]
    d = d.drop_duplicates("conId", keep="last")
    d["StrikePrice"] = pd.to_numeric(d.StrikePrice, errors="coerce")
    d["ExpirationDate"] = pd.to_datetime(d.ExpirationDate, errors="coerce")
    d = d.dropna(subset=["StrikePrice", "ExpirationDate"])
    if d.empty:
        raise ValueError("Aucune option (call/put) dans les définitions IB reçues.")

    t = ticks.copy() if ticks is not None else pd.DataFrame(columns=["conId"])
    if "conId" not in t.columns:
        raise ValueError(f"Valeurs IB inattendues : {list(t.columns)}")
    for champ in CHAMPS_TICK:
        if champ not in t.columns:
            t[champ] = np.nan
        t[champ] = pd.to_numeric(t[champ], errors="coerce")
    t = t.drop_duplicates("conId", keep="last").set_index("conId")

    df = d.join(t[CHAMPS_TICK], on="conId")

    # last() ignore les NaN : un contrat sans valeur reçue n'écrase donc pas
    # celle d'un contrat voisin du même strike, et n'ajoute pas de ligne.
    keys = ["ExpirationDate", "StrikePrice"]
    calls = df[df.right == "C"].groupby(keys)[CHAMPS_TICK].last().add_prefix("Call")
    puts = df[df.right == "P"].groupby(keys)[CHAMPS_TICK].last().add_prefix("Put")
    chain = calls.join(puts, how="outer").reset_index()

    quote_date = _quote_date(quote_date).to_pydatetime()

    if futures_price is None:
        futures_price = infer_futures_price(chain)
        if futures_price is None:
            raise ValueError(
                "Prix du future indéterminable : passe-le avec --futures-price."
            )
    futures_price = float(futures_price)

    T = ((chain.ExpirationDate - pd.Timestamp(quote_date)).dt.total_seconds()
         / (365.25 * 24 * 3600)).clip(lower=0)

    for side, opt in (("Call", "C"), ("Put", "P")):
        iv = pd.to_numeric(chain[f"{side}IV"], errors="coerce")
        # IB publie l'IV tantôt en décimal, tantôt en pourcentage. 18 ne peut pas
        # être 1 800 % de volatilité : au-delà de 3, c'est une échelle et non un
        # régime. Même test que databento_data, même raison.
        if iv.notna().any() and iv.max(skipna=True) > 3:
            iv = iv / 100.0
        manque = iv.isna() & chain[f"{side}Settle"].notna()
        if manque.any():
            iv.loc[manque] = [
                implied_vol(prix, futures_price, k, tt, rate, opt)
                for prix, k, tt in zip(chain.loc[manque, f"{side}Settle"],
                                       chain.loc[manque, "StrikePrice"], T[manque])
            ]
        chain[f"{side}IV"] = iv

        # Le gamma publié par IB est gardé tel quel ; là où il manque, Black-76 le
        # retrouve depuis l'IV. Sans ce repli, --gamma-source published et le
        # profil ne travailleraient pas sur le même périmètre de strikes.
        gamma = pd.to_numeric(chain[f"{side}Gamma"], errors="coerce")
        calcule = pd.Series(black76_gamma(futures_price, chain.StrikePrice, iv, T, rate),
                            index=chain.index)
        chain[f"{side}Gamma"] = gamma.where(gamma.notna(), calcule)

    chain["Calls"] = ""
    chain["Puts"] = ""
    chain = chain.reindex(columns=COLUMNS + COLONNES_GRECS)
    return _clean(chain), futures_price, quote_date


def selection_vif(chaine, budget_lignes=90):
    """Les contrats à garder souscrits en permanence, |gamma × OI| décroissant.

    Un critère géométrique — plus ou moins N strikes autour du spot — serait plus
    simple, mais dilapiderait des lignes sur des strikes sans open interest alors
    que le socle vient précisément de mesurer où le gamma se trouve.

    Le budget tombe mieux qu'on ne l'avait prévu : quatre-vingt-dix lignes font
    quarante-cinq strikes, soit environ ±2,5 %, exactement la grille que le CME
    liste sur une échéance hebdomadaire. Le vif ne couvre donc pas un morceau
    autour du spot, il couvre toute la chaîne listée de l'échéance proche, et le
    tri sert moins à choisir qu'à ordonner le recyclage quand le spot glisse.

    Quatre-vingt-dix et non cent : le future consomme une ligne, le recyclage en
    réclame quelques-unes le temps que les annulations soient prises en compte, et
    saturer le quota fait échouer les souscriptions suivantes en silence. Le
    budget est un paramètre, pas une constante — un compte avec des Quote Boosters
    en a davantage, et doit pouvoir s'en servir.

    Rend la même forme que perimetre() : la couche réseau ne connaît qu'un seul
    genre de contrat.
    """
    blocs = []
    for side, right in (("Call", "C"), ("Put", "P")):
        bloc = chaine[["ExpirationDate", "StrikePrice"]].copy()
        bloc["right"] = right
        gamma = pd.to_numeric(chaine.get(f"{side}Gamma"), errors="coerce").fillna(0.0)
        oi = pd.to_numeric(chaine.get(f"{side}OpenInt"), errors="coerce").fillna(0.0)
        bloc["poids"] = (gamma * oi).abs()
        blocs.append(bloc)

    tous = pd.concat(blocs, ignore_index=True)
    tous = tous[tous.poids > 0]
    # Le tri secondaire n'est pas cosmétique : à poids égaux, sans lui, deux
    # appels rendraient deux listes différentes et le recyclage annulerait puis
    # re-souscrirait les mêmes contrats pour rien.
    tous = tous.sort_values(["poids", "ExpirationDate", "StrikePrice", "right"],
                            ascending=[False, True, True, True])
    return tous.head(int(budget_lignes))[CONTRAT].reset_index(drop=True)


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
