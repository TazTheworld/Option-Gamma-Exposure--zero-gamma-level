"""Suivi du flux d'options par échantillonnage du CBOE (gratuit, sans compte).

Idée : on relève la chaîne toutes les N minutes. Entre deux relevés, pour chaque
contrat on connaît le volume échangé (différence des compteurs) et le prix du
dernier trade. Si ce trade est survenu dans la fenêtre, la fourchette du relevé
est contemporaine : on peut situer le trade dedans et en déduire le côté agressif.

    python flow_tracker.py ORCL --interval 300 --duration 6h
    python flow_tracker.py SPCX --interval 180 --out flux_spcx.csv

CE QUE ÇA DONNE
  - où le volume se concentre, strike par strike, au fil de la séance
  - une estimation du côté agressif (achat au-dessus du mid, vente en dessous)

CE QUE ÇA N'EST PAS
  - de vrais prints : on ne voit que le DERNIER trade de chaque fenêtre, pas
    chacun d'eux. Le volume de la fenêtre est attribué à ce côté, ce qui est une
    approximation d'autant plus grossière que la fenêtre est large.
  - du temps réel : les données CBOE sont différées d'environ 15 minutes.
  - fiable sur un contrat isolé. Le signal n'a de sens qu'agrégé sur beaucoup
    de contrats, là où les erreurs individuelles se compensent.

Pour de vrais prints, il faut le tape OPRA : Tradier (gratuit avec un compte),
Polygon ou Databento. Voir le README.
"""

import argparse
import os
import re
import time

import pandas as pd

import cboe_data
from greeks import CONTRACT_SIZE

# Un trade au-dessus de ce seuil dans la fourchette est jugé initié à l'achat
SEUIL_ACHAT = 0.60
SEUIL_VENTE = 0.40


def snapshot(ticker):
    """Relevé instantané : un contrat par ligne, avec volume et fourchette."""
    payload = cboe_data.fetch_json(ticker)
    data = payload["data"]
    rows = []
    for opt in data["options"]:
        parsed = cboe_data._OPTION_RE.match(opt["option"])
        if parsed is None:
            continue
        rows.append({
            "option": opt["option"],
            "StrikePrice": int(parsed["strike"]) / 1000.0,
            "cp": parsed["cp"],
            "expiry": parsed["exp"],
            "volume": opt.get("volume") or 0,
            "bid": opt.get("bid"),
            "ask": opt.get("ask"),
            "last": opt.get("last_trade_price"),
            "last_time": opt.get("last_trade_time"),
        })
    df = pd.DataFrame(rows)
    df["last_time"] = pd.to_datetime(df["last_time"], errors="coerce")
    return df, float(data["current_price"]), pd.to_datetime(payload["timestamp"])


def etat_marche(ts_utc):
    """(ouvert, heure_ET) — l'horodatage du payload CBOE est en UTC."""
    from zoneinfo import ZoneInfo

    ts = pd.Timestamp(ts_utc)
    ts = ts.tz_localize("UTC") if ts.tz is None else ts.tz_convert("UTC")
    et = ts.tz_convert(ZoneInfo("America/New_York"))
    heure = et.hour + et.minute / 60
    return (et.weekday() < 5 and 9.5 <= heure < 16), et


def classify(avant, apres, contract_size=CONTRACT_SIZE):
    """Compare deux relevés et renvoie le flux classifié de la fenêtre.

    Ne retient que les contrats dont le volume a augmenté ET dont l'horodatage
    du dernier trade a avancé : le trade est alors nouveau, et la fourchette du
    relevé lui est contemporaine.

    On compare les horodatages entre eux plutôt qu'à l'heure courante : le flux
    CBOE étant différé d'environ 15 minutes, last_trade_time est toujours en
    retard sur l'horodatage du payload, et une comparaison absolue ne matche
    jamais.

    `contract_size` convertit le volume en prime : 100 pour une action ou un ETF,
    mais tout autre chose sur un future — c'était codé en dur, donc faux dès qu'on
    suivait autre chose qu'une action.
    """
    m = apres.merge(avant[["option", "volume", "last_time"]], on="option",
                    how="left", suffixes=("", "_avant"))
    m["volume_avant"] = m["volume_avant"].fillna(0)
    m["delta"] = m["volume"] - m["volume_avant"]
    nouveau = m.last_time_avant.isna() | (m.last_time > m.last_time_avant)

    frais = m[(m.delta > 0) & nouveau].copy()
    if frais.empty:
        return frais.assign(pos=[], cote=[], prime=[])

    spread = frais.ask - frais.bid
    valide = (frais.bid > 0) & (spread > 0)
    frais = frais[valide].copy()
    if frais.empty:
        return frais.assign(pos=[], cote=[], prime=[])

    # 0 = au bid, 1 = à l'ask
    frais["pos"] = (frais.last - frais.bid) / (frais.ask - frais.bid)
    frais["cote"] = pd.cut(frais.pos, [-99, SEUIL_VENTE, SEUIL_ACHAT, 99],
                           labels=["vente", "milieu", "achat"])
    frais["prime"] = frais.delta * frais.last * contract_size
    return frais


def resume(flux):
    """Agrège le flux classifié : primes achetées/vendues par type d'option."""
    if flux.empty:
        return "  aucun trade classifiable dans cette fenêtre"

    lignes = []
    for cp, nom in (("C", "calls"), ("P", "puts")):
        part = flux[flux.cp == cp]
        if part.empty:
            continue
        achat = part.loc[part.cote == "achat", "prime"].sum()
        vente = part.loc[part.cote == "vente", "prime"].sum()
        lignes.append(f"  {nom:<6} achetes {achat/1e6:6.2f} M$ | vendus {vente/1e6:6.2f} M$"
                      f" | net {(achat-vente)/1e6:+6.2f} M$")

    top = (flux.groupby("StrikePrice")["prime"].sum().nlargest(3))
    lignes.append("  strikes les plus actifs : " +
                  ", ".join(f"{k:.0f} ({v/1e6:.2f} M$)" for k, v in top.items()))
    return "\n".join(lignes)


def signed_gex(chemin_flux, ticker, contract_size=CONTRACT_SIZE):
    """GEX signé par le flux mesuré, au lieu de la convention calls+/puts-.

    La convention postule que les dealers sont longs calls et shorts puts. Ici on
    le mesure : pour chaque strike, la position dealer est le miroir du flux client
    agressif.

        position_dealer = ventes_clients - achats_clients

    Les trades au milieu de la fourchette sont écartés, pas devinés — sans agresseur
    identifiable, leur sens est indéterminé.

    LIMITE ESSENTIELLE : cela mesure la variation d'inventaire de la séance, partant
    de zéro à l'ouverture, PAS le book existant. Un strike non traité pèse zéro ici,
    alors qu'il peut porter un gros open interest. Les deux lectures sont donc
    complémentaires : la convention décrit la structure accumulée, le flux décrit ce
    que les dealers ont pris aujourd'hui.
    """
    flux = pd.read_csv(chemin_flux)
    flux = flux[flux.cote.isin(["achat", "vente"])]        # le milieu est écarté
    if flux.empty:
        raise ValueError(f"aucun trade classifiable dans {chemin_flux}")

    signe = flux.cote.map({"vente": 1, "achat": -1})       # miroir du client
    flux = flux.assign(pos_dealer=flux.delta * signe)
    pos = flux.groupby(["StrikePrice", "cp"])["pos_dealer"].sum().unstack(fill_value=0)
    for c in ("C", "P"):
        if c not in pos:
            pos[c] = 0

    chaine, spot, asof = cboe_data.fetch_chain(ticker)
    # Le gamma vient de la chaîne courante : il doit dater du même jour que le flux,
    # sinon on pondère des contrats d'hier par le gamma d'aujourd'hui.
    jour_flux = pd.to_datetime(flux.snapshot).max()
    ecart = (pd.Timestamp(asof) - jour_flux).total_seconds() / 86400
    if ecart > 1:
        print(f"ATTENTION : le flux date du {jour_flux:%Y-%m-%d} et la chaîne du "
              f"{pd.Timestamp(asof):%Y-%m-%d} ({ecart:.0f} jours d'écart).\n"
              f"Le gamma appliqué n'est pas celui du jour des trades — résultat indicatif.\n")
    g = chaine.groupby("StrikePrice")[["CallGamma", "PutGamma"]].max()
    t = pos.join(g, how="inner")
    if t.empty:
        raise ValueError("aucun strike commun entre le flux et la chaîne courante")

    facteur = contract_size * spot ** 2 * 0.01
    t["gex_flux"] = (t.C * t.CallGamma + t.P * t.PutGamma) * facteur
    # même périmètre de strikes, mais signé par la convention
    conv = chaine.groupby("StrikePrice")[["CallGamma", "CallOpenInt", "PutGamma", "PutOpenInt"]].sum()
    conv = conv.loc[conv.index.intersection(t.index)]
    gex_conv = ((conv.CallGamma * conv.CallOpenInt - conv.PutGamma * conv.PutOpenInt) * facteur).sum()
    return t, spot, t.gex_flux.sum(), gex_conv


def afficher_signed(chemin_flux, ticker, contract_size=CONTRACT_SIZE):
    t, spot, gex_flux, gex_conv = signed_gex(chemin_flux, ticker, contract_size)
    ech = 1e9 if max(abs(gex_flux), abs(gex_conv)) >= 1e9 else 1e6
    unite = "Md$" if ech == 1e9 else "M$"
    print(f"{ticker} | spot {spot:.2f} | {len(t)} strikes tradés et classifiables\n")
    print(f"  GEX signé par le flux    : {gex_flux/ech:+,.2f} {unite}   "
          f"(inventaire pris aujourd'hui)")
    print(f"  GEX signé par convention : {gex_conv/ech:+,.2f} {unite}   "
          f"(structure accumulée, mêmes strikes)")
    accord = "MÊME SIGNE" if gex_flux * gex_conv > 0 else "SIGNES OPPOSÉS"
    print(f"  -> {accord}")
    if gex_flux * gex_conv < 0:
        print("     Le flux du jour contredit l'hypothèse conventionnelle : les dealers\n"
              "     ont pris l'inverse de ce que la structure suggère.")
    print(f"\n  contrats nets pris par les dealers : calls {t.C.sum():+,.0f}  "
          f"puts {t.P.sum():+,.0f}")
    print("\n  strikes les plus signants :")
    for k, r in t.reindex(t.gex_flux.abs().sort_values(ascending=False).index[:6]).iterrows():
        print(f"    {k:>8.2f}  {r.gex_flux/ech:+7.3f} {unite}   "
              f"dealers : {r.C:+,.0f} calls, {r.P:+,.0f} puts")
    return t


def _duree(txt):
    """'6h', '90m', '3600' -> secondes."""
    m = re.fullmatch(r"(\d+(?:\.\d+)?)\s*([hms]?)", str(txt).strip().lower())
    if not m:
        raise argparse.ArgumentTypeError(f"durée invalide : {txt}")
    val, unit = float(m.group(1)), m.group(2)
    return int(val * {"h": 3600, "m": 60, "s": 1, "": 1}[unit])


# main.py --watch attend la même écriture des durées : un seul analyseur, pour que
# "5m" ne veuille pas dire deux choses selon le script qui le lit.
duree = _duree


def track(ticker, interval=300, duration=6 * 3600, out=None, contract_size=CONTRACT_SIZE):
    """Boucle d'échantillonnage. Écrit le flux classifié au fil de l'eau."""
    out = out or f"flux_{ticker.lower()}.csv"
    avant, spot, ts = snapshot(ticker)
    debut = pd.Timestamp(ts).tz_localize(None) if ts.tz else pd.Timestamp(ts)
    ouvert, et = etat_marche(ts)
    print(f"{ticker} | spot {spot:.2f} | {len(avant)} contrats | premier relevé {debut:%H:%M:%S} UTC")
    print(f"heure de New York : {et:%H:%M} — marché {'ouvert' if ouvert else 'FERMÉ'}")
    if not ouvert:
        print("\nHors séance, volume et derniers trades restent figés sur la clôture\n"
              "précédente : aucun flux ne sera détecté. Lance entre 9h30 et 16h ET,\n"
              "et compte 15 minutes de plus, le flux CBOE étant différé d'autant.")
    print(f"\néchantillonnage toutes les {interval}s pendant {duration/3600:.1f}h -> {out}\n")

    fin = time.time() + duration
    total = 0
    while True:
        reste = fin - time.time()
        if reste < interval / 2:      # pas de fenêtre tronquée en fin de course
            break
        time.sleep(min(interval, reste))
        try:
            apres, spot, ts = snapshot(ticker)
        except Exception as err:
            print(f"  relevé manqué ({type(err).__name__}) — on continue")
            continue
        maintenant = pd.Timestamp(ts).tz_localize(None) if ts.tz else pd.Timestamp(ts)
        flux = classify(avant, apres, contract_size)
        if not flux.empty:
            flux = flux.assign(snapshot=maintenant, spot=spot)
            cols = ["snapshot", "spot", "option", "StrikePrice", "cp", "expiry",
                    "delta", "last", "bid", "ask", "pos", "cote", "prime"]
            flux[cols].to_csv(out, mode="a", header=not os.path.exists(out), index=False)
            total += len(flux)
        print(f"[{maintenant:%H:%M:%S}] spot {spot:.2f} | {len(flux)} contrats actifs")
        print(resume(flux))
        avant = apres

    print(f"\n{total} lignes de flux écrites dans {out}")
    return out


def main():
    p = argparse.ArgumentParser(description="Suivi du flux d'options (CBOE, gratuit)")
    p.add_argument("ticker", help="sous-jacent (ORCL, SPCX, _SPX...)")
    p.add_argument("--signed", metavar="FLUX.CSV",
                   help="ne pas échantillonner : lire un flux déjà collecté et en "
                        "déduire le signe réel de la position dealer, strike par strike")
    p.add_argument("--interval", type=_duree, default=300,
                   help="délai entre relevés (300, 5m...) — sous 3m le bruit domine")
    p.add_argument("--duration", type=_duree, default="6h", help="durée totale (6h, 90m...)")
    p.add_argument("--out", help="fichier CSV de sortie")
    p.add_argument("--contract-size", type=float, default=CONTRACT_SIZE,
                   help=f"multiplicateur du contrat (défaut : {CONTRACT_SIZE}) — les primes "
                        "et le GEX signé sont faux avec la mauvaise valeur")
    args = p.parse_args()
    if args.signed:
        afficher_signed(args.signed, args.ticker, args.contract_size)
        return
    if args.interval < 120:
        print("Attention : sous 2 minutes, les données différées du CBOE bougent peu "
              "et le bruit domine.\n")
    track(args.ticker, args.interval, args.duration, args.out, args.contract_size)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\ninterrompu")
