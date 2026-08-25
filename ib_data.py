"""Collecteur Interactive Brokers pour les options sur futures NQ.

IB ne sert pas une chaîne : il sert des contrats un par un, avec un plafond de
cent lignes de données simultanées. Tout ce module découle de cette contrainte.

Ce fichier ne contient que les fonctions pures — celles qui décident, assemblent
et fusionnent, et qui se vérifient sans réseau. C'est le patron de
databento_data.py, pour la même raison qui y est écrite : ce qui produit un
chiffre doit être testable hors ligne.

    echeances_utiles()  quelles échéances énumérer
    perimetre()         parmi les contrats cotés, lesquels demander
    build_chain()       définitions + valeurs -> chaîne au format du projet
    selection_vif()     les contrats à garder souscrits en permanence
    fusionner()         open interest du socle + IV du vif -> entrée d'analyser()

L'ordre compte. reqSecDefOptParams ne sert qu'à lister les échéances, son union
d'échéances étant exacte ; son union de STRIKES ne l'est pas, et croiser les deux
fabriquerait des dizaines de milliers de contrats jamais cotés. On énumère donc
les contrats réels échéance par échéance, avec reqContractDetails, et perimetre()
n'élague que ce qui est hors plage.

La connexion, l'énumération et la souscription vivent ailleurs : elles demandent
IB Gateway, donc elles ne se testent pas ici. Rien dans ce fichier n'importe
ib_async — `python main.py TSLA` ne doit pas payer une dépendance de courtier.

Conception : docs/superpowers/specs/2026-08-25-collecteur-ib-nq-design.md
"""

import math

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
#
# La seconde moitié — carnet et séance — arrive dans la MÊME souscription, sans
# generic tick, sans ligne ni requête supplémentaire. La jeter serait perdre une
# donnée gratuite, et snapshots.py dit pourquoi c'est un mauvais calcul : on
# archive le brut parce qu'une donnée non collectée est perdue pour toujours,
# alors qu'une colonne que rien ne lit encore ne coûte que quelques octets.
CHAMPS_TICK = [
    "OpenInt", "IV", "Gamma", "Delta", "Vega", "Theta", "Settle",
    "Bid", "Ask", "BidSize", "AskSize", "Vol", "LastSale",
]

# Les tailles au meilleur bid et au meilleur ask n'ont pas de place dans COLUMNS,
# qui suit le format historique du CBOE. Elles sont donc préservées à part du
# reindex : rien ne les lit aujourd'hui, mais sans elles une archive ne permettra
# jamais de reconstituer la pression du carnet a posteriori.
COLONNES_CARNET = [f"{cote}{champ}" for cote in ("Call", "Put")
                   for champ in ("BidSize", "AskSize")]

# Ce que le vif peut rafraîchir. L'open interest n'y est PAS, et c'est le coeur de
# la fusion : la chambre de compensation le calcule après la clôture et ne le
# publie qu'une fois par jour. Le figer en séance n'est pas une approximation,
# c'est la seule valeur qui existe.
CHAMPS_VIFS = ["IV", "Gamma"]


def _quote_date(valeur=None):
    """Date de valorisation normalisée, sans fuseau."""
    ts = pd.Timestamp(valeur) if valeur is not None else pd.Timestamp.now("UTC")
    if ts.tz is not None:
        ts = ts.tz_localize(None)
    return ts.normalize()


def echeances_utiles(echeances, quote_date=None, dte_max=30, dte_min=0):
    """Les échéances à énumérer, triées et dédoublonnées.

    C'est la seule chose pour laquelle reqSecDefOptParams est fiable : il rend
    l'union des échéances du sous-jacent, et cette union est exacte. Son union de
    strikes, elle, ne l'est pas — voir perimetre().

    Le résultat donne le nombre d'appels à reqContractDetails, un par échéance.
    Un doublon serait une requête payée pour rien, et l'ordre doit être
    reproductible pour que deux balayages successifs se ressemblent.

    `dte_max=None` garde toutes les échéances (le 'all' de main.py).
    """
    quote_date = _quote_date(quote_date)
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
    return gardees


def perimetre(contrats, prix, plage=0.2):
    """Parmi les contrats RÉELLEMENT cotés, ceux à demander.

    Cette fonction filtre, elle ne fabrique pas. La distinction est le fond du
    problème : reqSecDefOptParams rend l'union des strikes et l'union des
    échéances du sous-jacent, jamais les couples existants. Leur produit
    cartésien est un majorant — sur NQ à ±20 %, vingt-quatre mille contrats dont
    la vaste majorité n'a jamais été cotée, parce que le CME ne liste que
    vingt-cinq strikes autour du règlement sur les échéances hebdomadaires.
    Souscrire à ces fantômes coûterait des minutes pour récolter une erreur 200
    par contrat.

    On énumère donc les contrats existants d'abord — un reqContractDetails par
    échéance, le strike laissé indéfini, ce qui rend tous les contrats de cette
    échéance avec leurs conId — et cette fonction n'élague que ce qui est hors
    plage.

    `contrats` doit porter au moins StrikePrice ; conId, ExpirationDate et right
    sont conservés tels quels, le conId étant la seule chose que la couche réseau
    ne pourrait pas reconstruire.

    L'ordre du résultat est celui dans lequel les lots partiront : trié, donc
    reproductible d'un balayage à l'autre.
    """
    df = contrats.copy()
    df["StrikePrice"] = pd.to_numeric(df["StrikePrice"], errors="coerce")
    df = df.dropna(subset=["StrikePrice"])

    bas, haut = float(prix) * (1 - plage), float(prix) * (1 + plage)
    # Les bornes se calculent en flottant, donc fausses de quelques femtomètres :
    # 25 000 x 1,025 vaut 25 624,999999999996, ce qui EXCLUT le strike 25 625
    # pourtant demandé. Sans marge on perdrait le strike le plus éloigné — celui
    # qui borne le profil de gamma — et de façon imprévisible, l'erreur dépendant
    # du prix du future et changeant donc d'une séance à l'autre.
    marge = abs(float(prix)) * 1e-9
    df = df[(df.StrikePrice >= bas - marge) & (df.StrikePrice <= haut + marge)]

    tri = [c for c in ("ExpirationDate", "StrikePrice", "right") if c in df.columns]
    return df.sort_values(tri).reset_index(drop=True)


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
        # IB sert le prix du sous-jacent dans modelGreeks.undPrice, donc dans
        # CHAQUE tick d'option. La parité call-put ne sert plus que de filet,
        # pour une source qui ne le donnerait pas — le CME et Databento.
        futures_price = _undprice(ticks)
    if futures_price is None:
        futures_price = infer_futures_price(chain)
    if futures_price is None:
        raise ValueError(
            "Prix du future indéterminable : ni undPrice, ni parité call-put "
            "exploitable. Passe-le avec --futures-price."
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
    chain = chain.reindex(columns=COLUMNS + COLONNES_GRECS + COLONNES_CARNET)
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


# Cent lignes de données simultanées chez IB, par défaut. Le future en consomme
# une, et le recyclage quelques-unes le temps que les annulations soient prises en
# compte : on ne demande donc jamais le quota entier.
BUDGET_LIGNES = 90


def lots(contrats, taille=BUDGET_LIGNES):
    """Découpe le périmètre en paquets souscriptibles d'un coup.

    C'est le nombre de LOTS qui fixe le temps de balayage, pas le nombre de
    contrats : chaque lot coûte une attente de stabilisation, et cette attente ne
    dépend pas de sa taille. Les six mille sept cents contrats mesurés sur NQ
    font soixante-quinze lots, soit trois à cinq minutes.

    Un périmètre vide rend une liste vide — une échéance sans strike dans la
    plage n'est pas une panne. Une taille nulle, elle, en est une : elle
    bouclerait indéfiniment.
    """
    taille = int(taille)
    if taille < 1:
        raise ValueError(f"taille de lot absurde : {taille} (attendu : au moins 1)")
    if contrats is None or len(contrats) == 0:
        return []
    return [contrats.iloc[i:i + taille].reset_index(drop=True)
            for i in range(0, len(contrats), taille)]


# IB ne dit pas « pas de valeur » par NaN mais par -1 sur les prix. Laisser
# passer un -1 donnerait des primes negatives, un implied_vol calcule sur du
# vide, et un GEX faux sans que rien ne le signale. Les TAILLES, elles, valent
# legitimement zero : aucune quantite affichee est une information.
PRIX_ABSENT = -1.0


def _prix(valeur):
    """Prix IB -> flottant, avec -1 traduit en absence."""
    if valeur is None:
        return np.nan
    valeur = float(valeur)
    if math.isnan(valeur) or valeur == PRIX_ABSENT:
        return np.nan
    return valeur


def _taille(valeur):
    """Taille IB -> flottant. Zero est une valeur, pas une absence."""
    return np.nan if valeur is None else float(valeur)


def ligne_ticker(ticker):
    """Un Ticker d'ib_async -> une ligne au vocabulaire du projet.

    C'est la seule traduction entre les noms d'IB et ceux du format pivot, et
    c'est pourquoi elle est pure : tout le reste de la couche reseau n'est qu'une
    boucle autour d'elle, et se verifie contre TWS plutot que dans la suite.

    Trois pieges, tous releves au sondage du 25 aout 2026 :

      - l'open interest est dans `callOpenInterest` OU `putOpenInterest` selon le
        sens du contrat, jamais dans les deux ;
      - `modelGreeks` vaut None pour un contrat qui ne repond pas ;
      - `undPrice` voyage avec les grecs : le prix du future arrive dans chaque
        tick d'option, ce qui evite de le demander separement.
    """
    contrat = ticker.contract
    droit = str(getattr(contrat, "right", "") or "").upper()[:1]

    oi = ticker.callOpenInterest if droit == "C" else ticker.putOpenInterest
    grecs = ticker.modelGreeks

    def grec(nom):
        valeur = getattr(grecs, nom, None) if grecs is not None else None
        return np.nan if valeur is None else float(valeur)

    return {
        "conId": getattr(contrat, "conId", None),
        "OpenInt": _taille(oi),
        "IV": grec("impliedVol"),
        "Gamma": grec("gamma"),
        "Delta": grec("delta"),
        "Vega": grec("vega"),
        "Theta": grec("theta"),
        # Le reglement de la veille, dont infer_futures_price a besoin si
        # undPrice venait a manquer.
        "Settle": _prix(ticker.close),
        "Bid": _prix(ticker.bid),
        "Ask": _prix(ticker.ask),
        "BidSize": _taille(ticker.bidSize),
        "AskSize": _taille(ticker.askSize),
        "Vol": _taille(ticker.volume),
        "LastSale": _prix(ticker.last),
        "UndPrice": grec("undPrice"),
    }


def _undprice(ticks):
    """Le prix du sous-jacent tel qu'IB le sert avec les grecs, ou None.

    La mediane plutot que la derniere valeur : les ticks d'un meme balayage
    n'arrivent pas tous a la meme seconde, et un contrat isole peut porter un
    undPrice decale sans que rien ne le signale.
    """
    if ticks is None or "UndPrice" not in getattr(ticks, "columns", []):
        return None
    valeurs = pd.to_numeric(ticks["UndPrice"], errors="coerce").dropna()
    valeurs = valeurs[valeurs > 0]
    return None if valeurs.empty else float(valeurs.median())


def fusionner(socle, ticks_vif, spot):
    """Open interest du socle, IV et gamma du vif, spot du vif -> (df, spot).

    Rend exactement ce qu'analysis.analyser() prend en entrée : c'est tout
    l'objet de la fonction. Rien en aval n'a à savoir qu'il existe un socle et un
    vif — ni analysis, ni greeks, ni plots, ni history, ni validate.

    Le vif ne couvre qu'environ ±2,5 %. Partout ailleurs l'IV reste celle du
    socle, ce qui pèse peu : le gamma s'effondre loin de la monnaie, et l'IV y
    bouge peu. Un contrat que le vif porte mais que le socle ignore est écarté —
    on ne fabrique pas de ligne à partir d'un tick isolé.
    """
    df = socle.copy()
    if ticks_vif is None or len(ticks_vif) == 0:
        return df, float(spot)

    vif = ticks_vif.copy()
    vif["ExpirationDate"] = pd.to_datetime(vif["ExpirationDate"], errors="coerce")
    vif["StrikePrice"] = pd.to_numeric(vif["StrikePrice"], errors="coerce")
    vif["right"] = vif["right"].astype(str).str.upper().str[0]

    cle = pd.MultiIndex.from_arrays([df.ExpirationDate, df.StrikePrice])
    for side, right in (("Call", "C"), ("Put", "P")):
        part = vif[vif.right == right]
        if part.empty:
            continue
        for champ in CHAMPS_VIFS:
            if champ not in part.columns:
                continue
            valeurs = pd.to_numeric(part[champ], errors="coerce")
            # last() plutôt que first() : le dernier tick reçu est le bon. Le
            # groupby dédoublonne aussi, sans quoi reindex refuserait de servir.
            maj = valeurs.groupby([part.ExpirationDate, part.StrikePrice]).last().dropna()
            if maj.empty:
                continue
            nouvelles = maj.reindex(cle).to_numpy()
            colonne = f"{side}{champ}"
            df[colonne] = np.where(pd.isna(nouvelles), df[colonne].to_numpy(), nouvelles)

    return df, float(spot)


# ---=== Accès réseau — non testable hors ligne ===---

HOTE_DEFAUT, PORT_DEFAUT = "127.0.0.1", 7496
CLIENT_ID_DEFAUT = 17

# 3 = différé. Sans abonnement CME temps réel, le type 1 ne rend RIEN — pas
# d'erreur, juste des champs vides — alors que le différé sert tout, open
# interest compris. Le mode retenu est annoncé à l'écran : le projet n'accepte
# pas qu'une donnée dégradée passe en silence.
MARCHE_TEMPS_REEL, MARCHE_DIFFERE = 1, 3

# Les generic ticks demandés à chaque souscription. 101 porte l'open interest des
# options, 588 celui des futures : rien ne documente lequel IB retient pour un
# FOP, et les demander ensemble ne coûte aucune ligne supplémentaire.
TICKS_GENERIQUES = "101,588"


def connecter(hote=HOTE_DEFAUT, port=PORT_DEFAUT, client_id=CLIENT_ID_DEFAUT,
              differe=None):
    """Ouvre une session TWS en LECTURE SEULE.

    readonly=True n'est pas une précaution de style : le collecteur ne doit pas
    pouvoir passer d'ordre, même par un bug.

    Le mode de données effectif est écrit sur la sortie standard — une donnée
    différée qui passerait pour du temps réel fausserait toute lecture.
    """
    from ib_async import IB

    ib = IB()
    try:
        ib.connect(hote, port, clientId=client_id, timeout=15, readonly=True)
    except Exception as err:
        raise ValueError(
            f"Connexion à TWS impossible sur {hote}:{port} — {err}\n"
            "TWS ou IB Gateway tourne-t-il, et 'Enable ActiveX and Socket "
            "Clients' est-il coché dans Global Configuration > API > Settings ?"
        )

    mode = MARCHE_DIFFERE if differe else MARCHE_TEMPS_REEL
    ib.reqMarketDataType(mode)
    print(f"Connecté à {hote}:{port} — données "
          f"{'DIFFÉRÉES (~15 min)' if mode == MARCHE_DIFFERE else 'temps réel'}")
    return ib


def front_month(ib, produit="NQ", exchange="CME"):
    """Le future de première échéance : c'est lui que les options suivent."""
    from ib_async import Future

    details = ib.reqContractDetails(Future(produit, exchange=exchange))
    if not details:
        raise ValueError(f"Aucun future '{produit}' sur {exchange}.")
    proche = min(details, key=lambda d: d.contract.lastTradeDateOrContractMonth)
    return proche.contract


def enumerer(ib, futur, dte_max=30, dte_min=0, quote_date=None, exchange="CME",
             progres=True):
    """Les contrats d'options RÉELLEMENT cotés dans l'horizon demandé.

    Deux appels de nature différente, et l'ordre compte.

    `reqSecDefOptParams` d'abord, uniquement pour les échéances : il rend une
    entrée PAR CLASSE DE COTATION — quatorze sur NQ — et chaque classe ne porte
    qu'une échéance avec ses propres strikes. Ne lire que la première en fait
    manquer treize.

    `reqContractDetails` ensuite, une fois par échéance retenue, le strike laissé
    indéfini : IB rend alors tous les contrats de cette échéance avec leurs
    conId. C'est la seule façon d'obtenir les couples existants. Croiser les
    strikes et les échéances de reqSecDefOptParams fabriquerait un produit
    cartésien — 11 872 contrats là où il n'en existe que 6 696, la majorité
    jamais cotée.
    """
    from ib_async import FuturesOption

    params = ib.reqSecDefOptParams(futur.symbol, exchange, "FUT", futur.conId)
    if not params:
        raise ValueError(
            f"reqSecDefOptParams ne rend rien pour {futur.symbol}. "
            "Sans liste d'échéances, il n'y a rien à énumérer."
        )
    toutes = sorted({e for classe in params for e in classe.expirations})
    retenues = echeances_utiles(toutes, quote_date, dte_max, dte_min)
    if progres:
        print(f"{len(params)} classes de cotation, {len(toutes)} échéances, "
              f"{len(retenues)} dans l'horizon")

    lignes = []
    for exp in retenues:
        jour = pd.Timestamp(exp).strftime("%Y%m%d")
        details = ib.reqContractDetails(
            FuturesOption(futur.symbol, lastTradeDateOrContractMonth=jour,
                          exchange=exchange))
        for d in details:
            c = d.contract
            if str(c.right).upper()[:1] not in ("C", "P"):
                continue
            lignes.append({"conId": c.conId, "StrikePrice": float(c.strike),
                           "ExpirationDate": pd.Timestamp(exp),
                           "right": str(c.right).upper()[:1]})
        if progres:
            print(f"  {jour} : {len(details)} contrats")

    if not lignes:
        raise ValueError("Aucun contrat d'option énuméré sur l'horizon demandé.")
    return pd.DataFrame(lignes)


if __name__ == "__main__":
    # Sans réseau : ce que coûte un périmètre, en requêtes puis en lots.
    import sys

    prix = float(sys.argv[1]) if len(sys.argv) > 1 else 25_000.0
    plage = float(sys.argv[2]) if len(sys.argv) > 2 else 0.025

    # Grille telle que le CME la liste : vingt-cinq strikes de part et d'autre du
    # règlement sur les hebdomadaires, au pas de vingt-cinq points.
    ech = pd.date_range(pd.Timestamp.today().normalize(), periods=45, freq="D")
    interrogees = echeances_utiles(ech, dte_max=30)
    cotes = pd.DataFrame([
        {"conId": i, "ExpirationDate": d, "right": r,
         "StrikePrice": prix + n * 25.0}
        for i, (d, n, r) in enumerate(
            (d, n, r) for d in interrogees for n in range(-25, 26) for r in ("C", "P"))
    ])

    p = perimetre(cotes, prix, plage=plage)
    lots = -(-len(p) // 90)
    print(f"NQ à {prix:,.0f}, plage ±{plage:.1%}")
    print(f"  {len(interrogees)} requêtes reqContractDetails, une par échéance")
    print(f"  {len(cotes):,} contrats cotés -> {len(p):,} dans la plage")
    print(f"  {lots} lots de 90, soit environ {max(1, lots * 3 // 60)} min de balayage")
    print()
    print("Les contrats sont enumeres avant d'etre filtres, jamais l'inverse :")
    print("le produit cartesien des strikes et des echeances rendus par")
    print("reqSecDefOptParams compterait des milliers de contrats jamais cotes.")
