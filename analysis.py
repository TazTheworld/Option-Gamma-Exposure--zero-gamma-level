"""Cœur analytique : filtre d'échéance, expositions, murs, profil de gamma.

Tout ce qui produit un chiffre affiché vit ici, séparé du CLI (main.py) et des
graphiques (plots.py). Ces fonctions ne touchent ni au réseau, ni au disque, ni
à matplotlib : elles prennent une chaîne d'options et rendent des nombres, donc
elles sont testables directement — ce qui n'était pas le cas quand elles étaient
enfermées dans main().

L'entrée est toujours le format COLUMNS de cboe_data, quelle que soit la source
(CBOE, CME, Databento).
"""

from dataclasses import dataclass, field

import numpy as np
import pandas as pd

from greeks import (CONTRACT_SIZE, TRADING_DAYS, calc_charm_ex, calc_gamma_ex,
                    calc_vanna_ex)

# "iv" : gamma recalculé en Black-Scholes depuis la volatilité implicite.
# "published" : gamma tel que diffusé par la source (le CBOE le publie, pas le CME).
SOURCES_GAMMA = ("iv", "published")


# ---=== Filtre d'échéance ===---

def dte_calendaire(df, quote_date):
    """Jours calendaires jusqu'à l'échéance, une valeur par ligne."""
    return (df.ExpirationDate - pd.Timestamp(quote_date)).dt.days


def filtre_echeances(df, quote_date, dte_max=30, dte_min=0):
    """Ne garde que les échéances dans la fenêtre [dte_min, dte_max] en jours calendaires.

    Les chaînes CBOE portent plusieurs années d'échéances. Sans filtre, les LEAPS
    — strikes ronds à très gros OI — dominent les murs et tirent le zero gamma,
    alors qu'ils ne génèrent quasiment aucun flux de hedging à court terme.
    Le filtre s'applique avant tout calcul, pour que murs et profil de gamma
    portent sur le même périmètre.
    """
    dte = dte_calendaire(df, quote_date)
    garde = dte >= max(0, dte_min)      # une échéance passée n'a plus de gamma
    if dte_max is not None:
        garde &= dte <= dte_max
    if not garde.any():
        futures = dte[dte >= 0]
        if not len(futures):
            raise ValueError("aucune échéance future dans les données")
        raise ValueError(f"aucune échéance à {dte_max} jours ou moins "
                         f"(la plus proche est à {int(futures.min())} jours) — "
                         f"élargis avec --dte-max {int(futures.min())}, ou --dte-max all")
    return df[garde].copy()


CONVENTIONS_TEMPS = ("heures", "bourse")


def time_to_expiry(expirations, asof, convention="heures"):
    """Temps restant jusqu'à l'échéance, en années. Renvoie aussi le diviseur
    permettant de ramener une dérivée temporelle à la journée.

    "heures" (défaut) : temps réel restant jusqu'à 16h00 New York le jour de
        l'échéance, rapporté à 365 jours. C'est la convention du CBOE.
    "bourse" : jours ouvrés / 262 avec un plancher à 1 jour, la convention du
        script de référence de Perfiliev.

    Le plancher à 1 jour surestime lourdement les 0DTE — un 0DTE à 10h du matin,
    c'est 0,23 jour, pas 1 — et le gamma variant en 1/racine(T), l'écart est
    massif. Mesuré sur le SPX : gamma recalculé / gamma publié passe d'une médiane
    de 1,134 (77 % des contrats à plus de 10 % d'écart) à 1,000 (38 %).
    """
    exp = pd.to_datetime(pd.Series(expirations).values)
    if convention == "bourse":
        jours = np.busday_count(
            np.full(len(exp), pd.Timestamp(asof).date(), dtype="datetime64[D]"),
            exp.to_numpy().astype("datetime64[D]"))   # pandas ne caste pas en [D]
        return np.where(jours == 0, 1, jours) / TRADING_DAYS, TRADING_DAYS

    from zoneinfo import ZoneInfo
    # ExpirationDate porte déjà 16h00, entendues en heure de New York ; l'horodatage
    # de la source est en UTC. On aligne les deux avant de soustraire.
    ny = ZoneInfo("America/New_York")
    exp_utc = (pd.DatetimeIndex(exp).tz_localize(ny, nonexistent="shift_forward",
                                                 ambiguous=True).tz_convert("UTC")
               .tz_localize(None))
    maintenant = pd.Timestamp(asof)
    maintenant = maintenant.tz_convert("UTC").tz_localize(None) if maintenant.tz else maintenant
    restant = (exp_utc - maintenant).total_seconds().to_numpy()
    # une minute de plancher : à T strictement nul le gamma diverge
    return np.maximum(restant, 60.0) / (365.0 * 24 * 3600), 365.0


# ---=== Expositions par contrat ===---

def expositions(df, spot, T, contract_size=CONTRACT_SIZE, source="iv",
                jours_par_an=TRADING_DAYS):
    """Ajoute GEX, charm et vanna à chaque ligne, avec la convention de signe usuelle
    (dealers longs les calls, shorts les puts).

    `source` choisit d'où vient le gamma :

      - "iv"        : recalculé en Black-Scholes depuis la volatilité implicite ;
      - "published" : celui diffusé par la source de données.

    Ce choix ne pouvait pas être fait avant : le GEX par strike prenait le gamma
    publié pendant que le profil — donc le zero gamma — recalculait depuis l'IV.
    Deux estimateurs pour deux chiffres affichés côte à côte, avec un écart mesuré
    de 13 % sur le SPX à 30 jours et 29 % sur les échéances à un jour. Le profil,
    lui, ne PEUT être que recalculé : un gamma publié n'existe qu'au spot du
    moment, pas aux niveaux hypothétiques du profil. D'où le défaut sur "iv",
    seule option cohérente de bout en bout.

    Les colonnes GEXiv et GEXpublie sont conservées pour mesurer l'écart entre les
    deux (voir ecart_gamma).
    """
    if source not in SOURCES_GAMMA:
        raise ValueError(f"source de gamma inconnue : {source!r} (attendu : {SOURCES_GAMMA})")

    out = df.copy()
    out["T"] = np.asarray(T, dtype=float)

    iv_call = calc_gamma_ex(spot, out.StrikePrice, out.CallIV, out["T"],
                            0, 0, "call", out.CallOpenInt, contract_size)
    iv_put = calc_gamma_ex(spot, out.StrikePrice, out.PutIV, out["T"],
                           0, 0, "put", out.PutOpenInt, contract_size)
    # GEX = gamma unitaire * OI * taille du contrat * spot^2, ramené à un mouvement de 1%
    pub_call = out.CallGamma * out.CallOpenInt * contract_size * spot ** 2 * 0.01
    pub_put = out.PutGamma * out.PutOpenInt * contract_size * spot ** 2 * 0.01

    out["GEXiv"] = iv_call - iv_put
    out["GEXpublie"] = pub_call - pub_put
    if source == "published":
        out["CallGEX"], out["PutGEX"] = pub_call, -pub_put
    else:
        out["CallGEX"], out["PutGEX"] = iv_call, -iv_put
    out["TotalGamma"] = out.CallGEX + out.PutGEX

    # Charm et vanna : toujours depuis l'IV, aucune source ne les publie.
    # Le charm est une dérivée temporelle : son diviseur doit être celui de la
    # convention ayant servi à calculer T, sinon il est faux d'un facteur 365/262.
    for nom, fonction in (("Charm", calc_charm_ex), ("Vanna", calc_vanna_ex)):
        extra = {"jours_par_an": jours_par_an} if fonction is calc_charm_ex else {}
        c = fonction(spot, out.StrikePrice, out.CallIV, out["T"], out.CallOpenInt,
                     contract_size, **extra)
        p = fonction(spot, out.StrikePrice, out.PutIV, out["T"], out.PutOpenInt,
                     contract_size, **extra)
        out[f"Call{nom}"] = c
        out[f"Put{nom}"] = -p
        out[f"Total{nom}"] = c - p
    return out


def ecart_gamma(df):
    """(total IV, total publié, écart relatif) — None si la source ne publie pas de gamma.

    Sert de diagnostic : au-delà de quelques pour cent, les données différées ne
    décrivent plus le même book que le modèle. L'écart explose sur les 0-1 DTE.
    """
    publie, iv = df.GEXpublie.sum(), df.GEXiv.sum()
    if not np.isfinite(publie) or publie == 0:
        return None
    return iv, publie, (iv - publie) / abs(publie)


# ---=== Murs ===---

def murs(par_strike, spot, wall_range=0.15, oi_wall_range=0.30):
    """Strikes concentrant le plus de gamma (call = résistance, put = support),
    et leurs équivalents en open interest brut.

    Chaque mur est cherché du bon côté du spot. Sans cette contrainte les deux
    tombent sur le strike ATM — le gamma unitaire y est maximal, ce qui suffit à
    battre des strikes dix fois plus chargés en OI — et le résultat n'est alors
    qu'une paraphrase du spot.

    Les murs gamma restent malgré tout attirés vers la monnaie, d'où la bande
    plus large pour les murs en OI : ces concentrations-là sont plus lointaines.
    Sur une action les deux lectures coïncident souvent ; sur un indice l'écart
    est net, et elles répondent à des questions différentes — où le hedging mord
    le plus, et où les positions sont réellement accumulées.
    """
    def bande(demi_plage):
        return par_strike[(par_strike.index >= (1 - demi_plage) * spot)
                          & (par_strike.index <= (1 + demi_plage) * spot)]

    def sommet(sous_ensemble, colonne, sens):
        """Strike extrême de la colonne, ou None s'il n'y a rien à retenir.

        Un GEX call nul (ou put non négatif) veut dire qu'il n'y a pas de mur :
        idxmax renverrait alors le premier strike de la bande, ce qui n'a aucun sens.
        """
        if not len(sous_ensemble):
            return None
        serie = sous_ensemble[colonne]
        if sens == "max":
            return serie.idxmax() if serie.max() > 0 else None
        return serie.idxmin() if serie.min() < 0 else None

    gamma_band = bande(wall_range)
    oi_band = bande(oi_wall_range)
    return {
        "call_wall": sommet(gamma_band[gamma_band.index >= spot], "CallGEX", "max"),
        "put_wall": sommet(gamma_band[gamma_band.index <= spot], "PutGEX", "min"),
        "call_wall_oi": sommet(oi_band[oi_band.index >= spot], "CallOpenInt", "max"),
        "put_wall_oi": sommet(oi_band[oi_band.index <= spot], "PutOpenInt", "max"),
    }


# ---=== Profil de gamma et zero gamma ===---

def profil_gamma(df, levels, contract_size=CONTRACT_SIZE):
    """GEX net de chaque contrat à chaque niveau de spot : matrice (niveaux, contrats).

    Vectorisé sur les deux dimensions — l'ancienne version bouclait en Python sur
    les 60 niveaux, soit 120 appels sur toute la chaîne.

    Le gamma est nécessairement recalculé depuis l'IV : à un niveau hypothétique,
    aucun gamma publié n'existe. La volatilité implicite est tenue constante, ce
    qui est l'hypothèse usuelle et sous-estime la réaction réelle (en pratique la
    vol monte quand le spot baisse).
    """
    S = np.asarray(levels, dtype=float).reshape(-1, 1)
    K, T = df.StrikePrice.values, df["T"].values
    call_ex = calc_gamma_ex(S, K, df.CallIV.values, T, 0, 0, "call",
                            df.CallOpenInt.values, contract_size)
    put_ex = calc_gamma_ex(S, K, df.PutIV.values, T, 0, 0, "put",
                           df.PutOpenInt.values, contract_size)
    return call_ex - put_ex


def croisements_zero(levels, profile):
    """Tous les niveaux où le gamma total change de signe, interpolés linéairement."""
    levels, profile = np.asarray(levels, float), np.asarray(profile, float)
    sortie = []
    for i in np.where(np.diff(np.sign(profile)))[0]:
        y0, y1 = profile[i], profile[i + 1]
        x0, x1 = levels[i], levels[i + 1]
        if y1 == y0:                      # deux zéros consécutifs : rien à interpoler
            continue
        sortie.append(x1 - (x1 - x0) * y1 / (y1 - y0))
    return sortie


def find_zero_gamma(levels, profile, spot=None):
    """Niveau de spot où le gamma total change de signe.

    Quand le profil croise zéro plusieurs fois — ce qui arrive dès que les ailes
    sont bruyantes — on retient le croisement le plus PROCHE DU SPOT : c'est lui
    qui délimite le régime dans lequel le marché se trouve effectivement. L'ancienne
    version prenait le premier croisement de la fenêtre, donc le plus bas ; sur un
    profil croisant en 91, 94 et 106 avec un spot à 100, elle annonçait 91,5 alors
    que la bascule se joue à 106.

    Sans spot, on garde l'ancien comportement (premier croisement).
    """
    croisements = croisements_zero(levels, profile)
    if not croisements:
        return None
    if spot is None:
        return croisements[0]
    return min(croisements, key=lambda x: abs(x - spot))


def pick_scale(values):
    """Choisit l'unité d'affichage (milliards ou millions) selon l'ordre de grandeur."""
    values = np.asarray([v for v in np.ravel(values) if v is not None and np.isfinite(v)])
    peak = np.max(np.abs(values)) if values.size else 0
    return (1e9, "milliards") if peak >= 1e9 else (1e6, "millions")


def is_third_friday(d):
    return d.weekday() == 4 and 15 <= d.day <= 21


# ---=== Assemblage ===---

@dataclass
class Analyse:
    """Tout ce qu'une séance produit : les chiffres, puis de quoi les tracer."""
    ticker: str
    spot: float
    quote_date: object
    contract_size: float
    dte_max: object
    dte_min: int
    source_gamma: str
    time_convention: str
    df: pd.DataFrame                 # chaîne filtrée, avec les colonnes d'exposition
    par_strike: pd.DataFrame         # agrégat par strike
    levels: np.ndarray
    profiles: dict
    total_gex: float
    total_charm: float
    total_vanna: float
    zero_gamma: object
    croisements: list
    from_strike: float
    to_strike: float
    call_wall: object = None
    put_wall: object = None
    call_wall_oi: object = None
    put_wall_oi: object = None
    ecart_gamma: object = None       # (iv, publié, relatif) ou None
    part_courtes: dict = field(default_factory=dict)   # poids des 0-1 DTE

    @property
    def horizon(self):
        return "toutes échéances" if self.dte_max is None else f"<= {self.dte_max}j"

    @property
    def decimals(self):
        return 4 if self.spot < 10 else 2      # les paires FX se lisent en pips


def poids_echeances_courtes(df, quote_date, seuil_jours=1):
    """Part du GEX et du charm portée par les échéances à 0-1 jour.

    Le gamma publié par la source et celui recalculé depuis l'IV s'accordent bien
    au-delà de quelques jours, mais divergent violemment sur les 0-1 DTE (médiane
    25 % sur le SPX, 82 % des contrats à plus de 10 % d'écart). C'est intrinsèque :
    près de l'échéance le gamma explose et dépend du spot à la minute, que des
    données différées ne donnent pas. On ne corrige pas, on signale.

    Le charm y est bien plus exposé que le gamma, puisqu'il varie en 1/T : sur le
    SPX, les 0-1 DTE pèsent 17 % du GEX mais 66 % du charm.
    """
    proches = dte_calendaire(df, quote_date) <= seuil_jours
    if not proches.any():
        return {}
    parts = {}
    for nom, colonne in (("du GEX", "TotalGamma"), ("du charm", "TotalCharm")):
        total = df[colonne].sum()
        if total:
            parts[nom] = abs(df.loc[proches, colonne].sum()) / abs(total)
    return parts


def analyser(df, spot, quote_date, ticker="?", contract_size=CONTRACT_SIZE,
             dte_max=30, dte_min=0, plage=0.2, wall_range=0.15, oi_wall_range=0.30,
             source_gamma="iv", time_convention="heures", n_niveaux=60):
    """Chaîne d'options brute -> tous les chiffres de la séance.

    Fonction pure : ni réseau, ni disque, ni graphique. C'est le point d'entrée
    testable du projet.
    """
    df = filtre_echeances(df, quote_date, dte_max, dte_min)
    T, jours_par_an = time_to_expiry(df.ExpirationDate, quote_date, time_convention)
    df = expositions(df, spot, T, contract_size, source_gamma, jours_par_an)

    par_strike = df.groupby("StrikePrice")[
        ["CallGEX", "PutGEX", "TotalGamma", "CallOpenInt", "PutOpenInt",
         "TotalCharm", "TotalVanna"]].sum()

    from_strike, to_strike = (1 - plage) * spot, (1 + plage) * spot
    levels = np.linspace(from_strike, to_strike, n_niveaux)

    net = profil_gamma(df, levels, contract_size)      # (niveaux, contrats)
    prochaine = df.ExpirationDate.min()
    troisiemes_vendredis = df.ExpirationDate[[is_third_friday(x) for x in df.ExpirationDate]]
    masques = {
        "All Expiries": np.ones(len(df), dtype=bool),
        "Ex-Next Expiry": (df.ExpirationDate != prochaine).values,
    }
    # Avec un --dte-max serré il peut ne rester aucun 3e vendredi : la courbe
    # serait alors identique à "All Expiries" et se superposerait à elle.
    if len(troisiemes_vendredis):
        masques["Ex-Next Monthly Expiry"] = (
            df.ExpirationDate != troisiemes_vendredis.min()).values
    profiles = {nom: net[:, masque].sum(axis=1) for nom, masque in masques.items()}

    profil = profiles["All Expiries"]
    croisements = croisements_zero(levels, profil)

    return Analyse(
        ticker=ticker, spot=spot, quote_date=quote_date, contract_size=contract_size,
        dte_max=dte_max, dte_min=dte_min, source_gamma=source_gamma,
        time_convention=time_convention,
        df=df, par_strike=par_strike, levels=levels, profiles=profiles,
        total_gex=df.TotalGamma.sum(), total_charm=df.TotalCharm.sum(),
        total_vanna=df.TotalVanna.sum(),
        zero_gamma=find_zero_gamma(levels, profil, spot), croisements=croisements,
        from_strike=from_strike, to_strike=to_strike,
        ecart_gamma=ecart_gamma(df),
        part_courtes=poids_echeances_courtes(df, quote_date),
        **murs(par_strike, spot, wall_range, oi_wall_range),
    )
