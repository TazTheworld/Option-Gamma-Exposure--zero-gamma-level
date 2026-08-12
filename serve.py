"""Interface web locale pour lire le GEX, en http.server de la bibliothèque standard.

    python serve.py                 # http://127.0.0.1:8000
    python serve.py --port 8080
    python serve.py --no-browser

Aucune dépendance nouvelle : ni Flask, ni bundler, ni CDN. La page est du HTML,
du CSS et du JavaScript sans framework, et les graphiques sont du SVG construit à
la main — le projet tient en cinq paquets et ce serveur n'en ajoute aucun.

Le serveur ne fait que deux choses : servir les fichiers de web/, et exposer
analysis.analyser() en JSON. Tout le calcul reste dans analysis.py, donc la page
ne peut pas afficher un chiffre que les tests ne couvrent pas.

La chaîne brute est gardée en mémoire quelques minutes par sous-jacent : changer
d'horizon ou de source de gamma recalcule localement, sans retélécharger.
"""

import argparse
import json
import math
import os
import threading
import time
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import pandas as pd

import analysis
import cboe_data
import snapshots
from cme_data import CONTRACT_SIZES
from greeks import CONTRACT_SIZE

RACINE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "web")
DUREE_CACHE = 180          # secondes : le flux CBOE est de toute façon différé de 15 min

_cache = {}
_verrou = threading.Lock()

TYPES = {".html": "text/html; charset=utf-8", ".css": "text/css; charset=utf-8",
         ".js": "text/javascript; charset=utf-8", ".svg": "image/svg+xml",
         ".ico": "image/x-icon", ".json": "application/json"}


# ---=== Données ===---

def chaine(ticker, replay=None):
    """(df, spot, quote_date, ticker_affiche, marche), depuis le cache si possible."""
    if replay:
        df, spot, quote_date, marche = snapshots.charger(replay)
        return df, spot, quote_date, ticker.lstrip("_").upper(), marche

    cle = ticker.upper()
    with _verrou:
        entree = _cache.get(cle)
        if entree and time.time() - entree[0] < DUREE_CACHE:
            return entree[1]

    df, spot, quote_date, marche = cboe_data.fetch_chain_et_marche(ticker)
    resultat = (df, spot, quote_date, cboe_data.cboe_symbol(ticker).lstrip("_"), marche)
    with _verrou:
        _cache[cle] = (time.time(), resultat)
    return resultat


def _nombre(valeur):
    """numpy -> JSON. NaN et infini deviennent null plutôt que du JSON invalide."""
    if valeur is None:
        return None
    try:
        f = float(valeur)
    except (TypeError, ValueError):
        return None
    return f if math.isfinite(f) else None


def payload(a):
    """Une Analyse -> le dictionnaire que la page consomme."""
    par_strike = a.par_strike
    ecart = a.ecart_gamma
    return {
        "ticker": a.ticker,
        "spot": _nombre(a.spot),
        "date": a.quote_date.strftime("%Y-%m-%d %H:%M"),
        "contract_size": _nombre(a.contract_size),
        "horizon": a.horizon,
        "dte_max": a.dte_max,
        "dte_min": a.dte_min,
        "source_gamma": a.source_gamma,
        "time_convention": a.time_convention,
        "regime_vol": a.regime_vol,
        "decimals": a.decimals,
        "n_strikes": int(a.df.StrikePrice.nunique()),
        "n_expiries": int(a.df.ExpirationDate.nunique()),

        "total_gex": _nombre(a.total_gex),
        "total_charm": _nombre(a.total_charm),
        "total_vanna": _nombre(a.total_vanna),
        "zero_gamma": _nombre(a.zero_gamma),
        "croisements": [_nombre(x) for x in a.croisements],
        "call_wall": _nombre(a.call_wall),
        "put_wall": _nombre(a.put_wall),
        "call_wall_oi": _nombre(a.call_wall_oi),
        "put_wall_oi": _nombre(a.put_wall_oi),
        "from_strike": _nombre(a.from_strike),
        "to_strike": _nombre(a.to_strike),

        "ecart_gamma": None if not ecart else {
            "iv": _nombre(ecart[0]), "publie": _nombre(ecart[1]),
            "relatif": _nombre(ecart[2]),
        },
        "part_courtes": {nom: _nombre(p) for nom, p in a.part_courtes.items()},
        "marche": {champ: _nombre(valeur) for champ, valeur in (a.marche or {}).items()},
        "gex_sur_volume": _nombre(a.gex_sur_volume),

        "strikes": [_nombre(k) for k in par_strike.index],
        "par_strike": {
            "total": [_nombre(v) for v in par_strike.TotalGamma],
            "total_titres": [_nombre(v) for v in par_strike.TotalGammaTitres],
            "call": [_nombre(v) for v in par_strike.CallGEX],
            "put": [_nombre(v) for v in par_strike.PutGEX],
            "call_oi": [_nombre(v) for v in par_strike.CallOpenInt],
            "put_oi": [_nombre(v) for v in par_strike.PutOpenInt],
            "oi_net": [_nombre(c - p) for c, p in zip(par_strike.CallOpenInt,
                                                      par_strike.PutOpenInt)],
            "charm": [_nombre(v) for v in par_strike.TotalCharm],
            "vanna": [_nombre(v) for v in par_strike.TotalVanna],
            "delta": [_nombre(v) for v in par_strike.TotalDelta],
            "vega": [_nombre(v) for v in par_strike.TotalVega],
            "call_iv": [_nombre(v) for v in par_strike.CallIV],
            "put_iv": [_nombre(v) for v in par_strike.PutIV],
        },
        "levels": [_nombre(x) for x in a.levels],
        "profiles": {nom: [_nombre(v) for v in valeurs]
                     for nom, valeurs in a.profiles.items()},
    }


_cache_reference = {}


def reference_du_jour(ticker, quote_date, reglages, strikes):
    """Expositions par strike au PREMIER relevé archivé de la même séance.

    C'est ce qui permet de tracer une mèche derrière chaque barre : non pas où
    l'exposition d'un strike est maintenant, mais d'où elle vient depuis ce matin.
    L'open interest ne bougeant qu'une fois par jour, ce qui se déplace en séance
    vient du spot et de la volatilité — et c'est justement ce qu'on veut voir.

    Renvoie None s'il n'y a pas d'autre relevé du jour : on ne compare pas une
    séance à elle-même, et surtout pas à la veille, où l'OI a changé.
    """
    jour = pd.Timestamp(quote_date).strftime("%Y-%m-%d")
    archives = [f for f in snapshots.lister(ticker)
                if os.path.basename(f).startswith(jour)]
    if len(archives) < 2:
        return None
    plus_ancienne = archives[0]

    cle = (plus_ancienne, tuple(sorted(reglages.items())))
    with _verrou:
        garde = _cache_reference.get(cle)
    if garde is None:
        df, spot, qd, marche = snapshots.charger(plus_ancienne)
        a = analysis.analyser(df, spot=spot, quote_date=qd, ticker=ticker,
                              marche=marche, **reglages)
        garde = {
            "date": qd.strftime("%H:%M"),
            "par_strike": a.par_strike,
            "spot": a.spot,
            "zero_gamma": a.zero_gamma,
        }
        with _verrou:
            _cache_reference[cle] = garde

    table = garde["par_strike"]
    colonnes = {"total": "TotalGamma", "total_titres": "TotalGammaTitres",
                "oi_net": None, "charm": "TotalCharm", "vanna": "TotalVanna",
                "delta": "TotalDelta", "vega": "TotalVega"}

    def aligner(colonne):
        if colonne is None:
            serie = table.CallOpenInt - table.PutOpenInt
        else:
            serie = table[colonne]
        return [_nombre(serie.get(k)) for k in strikes]

    return {
        "heure": garde["date"],
        "spot": _nombre(garde["spot"]),
        "zero_gamma": _nombre(garde["zero_gamma"]),
        "par_strike": {nom: aligner(col) for nom, col in colonnes.items()},
    }


def analyser(params):
    """Paramètres de requête -> payload. Lève ValueError avec un message lisible."""
    def flottant(nom, defaut):
        valeur = params.get(nom, [None])[0]
        return defaut if valeur in (None, "") else float(valeur)

    def texte(nom, defaut, permis=None):
        valeur = params.get(nom, [None])[0] or defaut
        if permis and valeur not in permis:
            raise ValueError(f"{nom} : {valeur!r} inconnu (attendu : {', '.join(permis)})")
        return valeur

    ticker = (params.get("ticker", ["SPCX"])[0] or "SPCX").strip()
    replay = params.get("replay", [None])[0] or None

    brut = params.get("dte_max", ["30"])[0]
    dte_max = None if str(brut).lower() in ("all", "toutes", "") else int(brut)

    df, spot, quote_date, affiche, marche = chaine(ticker, replay)
    contract_size = CONTRACT_SIZES.get(ticker.upper(), CONTRACT_SIZE) if replay else CONTRACT_SIZE

    reglages = dict(
        contract_size=contract_size, dte_max=dte_max,
        dte_min=int(flottant("dte_min", 0)), plage=flottant("range", 0.2),
        wall_range=flottant("wall_range", 0.15),
        oi_wall_range=flottant("oi_wall_range", 0.30),
        source_gamma=texte("gamma_source", "iv", analysis.SOURCES_GAMMA),
        time_convention=texte("time_convention", "heures", analysis.CONVENTIONS_TEMPS),
        regime_vol=texte("vol_regime", "sticky-strike", analysis.REGIMES_VOL))

    a = analysis.analyser(df, spot=spot, quote_date=quote_date, ticker=affiche,
                          marche=marche, **reglages)
    sortie = payload(a)
    # La référence est analysée aux MÊMES réglages : comparer deux horizons ou
    # deux sources de gamma ne dirait rien du déplacement intraséance.
    sortie["reference"] = reference_du_jour(affiche, quote_date, reglages,
                                            list(a.par_strike.index))
    return sortie


def liste_snapshots():
    sortie = []
    for chemin in snapshots.lister():
        ticker = os.path.basename(os.path.dirname(chemin))
        nom = os.path.basename(chemin).split(".")[0]
        sortie.append({"chemin": chemin.replace("\\", "/"), "ticker": ticker, "date": nom})
    return sortie


def historique(ticker=None):
    import history
    try:
        df = history.load(history.DEFAUT, ticker)
    except FileNotFoundError:
        return []
    df = df.where(df.notna(), None)
    return json.loads(df.to_json(orient="records"))


def flux_disponibles():
    """Fichiers de flux collectés par flow_tracker, dans le dossier courant."""
    import glob
    return sorted(os.path.basename(f) for f in glob.glob("flux_*.csv"))


def signe_du_flux(ticker, fichier):
    """Position dealer déduite du flux observé, comparée à la convention.

    La convention postule que les dealers sont longs les calls et shorts les puts.
    Ici on le mesure : pour chaque strike, leur position est le miroir du flux
    client agressif. Les deux lectures ne répondent pas à la même question — la
    convention décrit la structure accumulée, le flux ce qu'ils ont pris ce jour.
    """
    import flow_tracker

    if fichier not in flux_disponibles():
        raise ValueError(f"fichier de flux inconnu : {fichier!r}")
    t, spot, gex_flux, gex_conv = flow_tracker.signed_gex(fichier, ticker)
    t = t.sort_values("gex_flux", key=abs, ascending=False)
    return {
        "ticker": ticker, "fichier": fichier, "spot": _nombre(spot),
        "gex_flux": _nombre(gex_flux), "gex_convention": _nombre(gex_conv),
        "meme_signe": bool(gex_flux * gex_conv > 0),
        "n_strikes": int(len(t)),
        "strikes": [{"strike": _nombre(k), "gex_flux": _nombre(r.gex_flux),
                     "calls": _nombre(r.C), "puts": _nombre(r.P)}
                    for k, r in t.head(12).iterrows()],
    }


# ---=== Serveur ===---

class Handler(BaseHTTPRequestHandler):
    server_version = "GEX/1.1"

    def log_message(self, format, *args):     # noqa: A002 — signature imposée
        if not self.path.startswith("/static"):
            print(f"  {self.command} {self.path}")

    def _envoyer(self, code, corps, type_mime, cache=False):
        self.send_response(code)
        self.send_header("Content-Type", type_mime)
        self.send_header("Content-Length", str(len(corps)))
        # Pas de cache sur les données : chaque relevé doit être frais.
        self.send_header("Cache-Control", "public, max-age=3600" if cache else "no-store")
        self.end_headers()
        self.wfile.write(corps)

    def _json(self, donnees, code=200):
        self._envoyer(code, json.dumps(donnees, allow_nan=False).encode("utf-8"),
                      "application/json; charset=utf-8")

    def _fichier(self, nom):
        chemin = os.path.normpath(os.path.join(RACINE, nom.lstrip("/")))
        if not chemin.startswith(RACINE) or not os.path.isfile(chemin):
            self._envoyer(404, b"introuvable", "text/plain; charset=utf-8")
            return
        with open(chemin, "rb") as f:
            corps = f.read()
        mime = TYPES.get(os.path.splitext(chemin)[1], "application/octet-stream")
        self._envoyer(200, corps, mime, cache=nom.startswith("/static"))

    def do_GET(self):
        route = urlparse(self.path)
        params = parse_qs(route.query)

        if route.path in ("/", "/index.html"):
            return self._fichier("/index.html")
        if route.path.startswith("/static/"):
            return self._fichier(route.path[len("/static"):])

        if route.path == "/api/analyse":
            try:
                return self._json(analyser(params))
            except ValueError as err:
                return self._json({"erreur": str(err)}, 400)
            except FileNotFoundError as err:
                return self._json({"erreur": str(err)}, 404)
            except Exception as err:      # le réseau, surtout
                return self._json({"erreur": f"{type(err).__name__} : {err}"}, 500)

        if route.path == "/api/snapshots":
            return self._json(liste_snapshots())
        if route.path == "/api/history":
            return self._json(historique(params.get("ticker", [None])[0]))

        if route.path == "/api/flux":
            fichier = params.get("fichier", [None])[0]
            if not fichier:
                return self._json({"fichiers": flux_disponibles()})
            try:
                return self._json(signe_du_flux(params.get("ticker", ["SPCX"])[0], fichier))
            except ValueError as err:
                return self._json({"erreur": str(err)}, 400)
            except Exception as err:
                return self._json({"erreur": f"{type(err).__name__} : {err}"}, 500)

        self._envoyer(404, b"introuvable", "text/plain; charset=utf-8")


def main():
    p = argparse.ArgumentParser(description="Interface web locale du gamma exposure")
    p.add_argument("--port", type=int, default=8000)
    p.add_argument("--host", default="127.0.0.1",
                   help="127.0.0.1 par défaut : le serveur n'écoute que la machine locale")
    p.add_argument("--no-browser", action="store_true", help="ne pas ouvrir le navigateur")
    args = p.parse_args()

    serveur = ThreadingHTTPServer((args.host, args.port), Handler)
    url = f"http://{args.host}:{args.port}"
    print(f"Interface GEX sur {url}   (Ctrl+C pour arrêter)")
    if not args.no_browser:
        threading.Timer(0.5, lambda: webbrowser.open(url)).start()
    try:
        serveur.serve_forever()
    except KeyboardInterrupt:
        print("\narrêté")
        serveur.server_close()


if __name__ == "__main__":
    main()
