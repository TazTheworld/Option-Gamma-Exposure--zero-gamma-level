"""Récupération d'une chaîne d'options sur futures depuis Barchart.

Deux vues sont nécessaires et fusionnées sur (strike, type) :
  - volatility-greeks : IV, delta, gamma
  - options           : volume et open interest  <- indispensable au calcul de GEX

Le résultat est un CSV directement lisible par cme_data.py, donc par le pipeline :

    python Scrap-data.py E6U26 --expiry aug-26 --out barchart_6E.csv
    python main.py 6E --cme barchart_6E.csv --expiry 2026-08-28

Barchart rend ses tableaux dans un shadow DOM (<bc-data-grid>) : le texte n'est
pas accessible par .text, il faut descendre dans le shadowRoot. Le site sert par
ailleurs des cellules vides aux navigateurs automatisés selon l'adresse IP — le
script échoue alors bruyamment plutôt que d'écrire un fichier vide.
"""

import argparse
import sys
import time

import pandas as pd
from selenium import webdriver
from selenium.webdriver.chrome.options import Options
from selenium.webdriver.common.by import By

BASE = "https://www.barchart.com/futures/quotes/{sym}/{vue}/{exp}?futuresOptionsView=merged"

# Extrait les lignes de toutes les <bc-data-grid> de la page, en-têtes compris
JS_GRILLES = r"""
const out = [];
document.querySelectorAll('bc-data-grid').forEach((g, gi) => {
  const sr = g.shadowRoot; if (!sr) return;
  const heads = [...sr.querySelectorAll('div._header_cell')]
                  .map(e => e.textContent.trim().split('\n')[0]);
  const cells = [...sr.querySelectorAll('div._cell')].map(e => e.textContent.trim());
  const n = heads.length; if (!n) return;
  const rows = [];
  for (let i = 0; i + n <= cells.length; i += n) rows.push(cells.slice(i, i + n));
  out.push({index: gi, heads: heads, rows: rows.filter(r => r.some(c => c.length > 0))});
});
return out;
"""

CONSENT = ["#CybotCookiebotDialogBodyButtonDecline",
           "#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll"]


def make_driver(headless=True):
    opts = Options()
    args = ["--no-sandbox", "--disable-dev-shm-usage", "--window-size=1920,4000",
            "--disable-blink-features=AutomationControlled", "--log-level=3"]
    if headless:
        args.insert(0, "--headless=new")
    for a in args:
        opts.add_argument(a)
    opts.add_argument("user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                      "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36")
    opts.add_experimental_option("excludeSwitches", ["enable-automation"])
    driver = webdriver.Chrome(options=opts)
    driver.set_page_load_timeout(90)
    driver.execute_cdp_cmd(
        "Page.addScriptToEvaluateOnNewDocument",
        {"source": "Object.defineProperty(navigator,'webdriver',{get:()=>undefined})"})
    return driver


def scrape_view(driver, symbol, expiry, vue, wait=12):
    """Renvoie un DataFrame des lignes de la vue, avec une colonne Type."""
    url = BASE.format(sym=symbol, vue=vue, exp=expiry)
    print(f"  -> {url}")
    driver.get(url)
    time.sleep(6)
    for sel in CONSENT:
        try:
            driver.execute_script("arguments[0].click();",
                                  driver.find_element(By.CSS_SELECTOR, sel))
            break
        except Exception:
            continue
    time.sleep(wait)
    # Les grilles sont chargées à l'affichage : on les fait défiler
    for y in (800, 1600, 2400):
        driver.execute_script(f"window.scrollTo(0,{y});")
        time.sleep(3)

    grids = driver.execute_script(JS_GRILLES)
    frames = []
    for g in grids:
        if not g["rows"]:
            continue
        block = pd.DataFrame(g["rows"], columns=g["heads"])
        if "Type" not in block.columns:
            # Barchart sépare les tables : la première porte les calls, la seconde les puts
            block["Type"] = "Call" if g["index"] == 0 else "Put"
        frames.append(block)

    if not frames:
        vides = sum(1 for g in grids if not g["rows"])
        raise RuntimeError(
            f"Aucune donnée dans la vue '{vue}' ({len(grids)} grilles, {vides} vides).\n"
            "Les en-têtes chargent mais les cellules reviennent vides : Barchart sert la\n"
            "page sans valeurs à ce navigateur. Essaie --visible, ou depuis une autre\n"
            "connexion. Ce n'est pas un problème d'URL ni d'attente."
        )
    return pd.concat(frames, ignore_index=True)


def _num(series):
    """'1,234.5' / '12.3%' / 'N/A' -> float."""
    return pd.to_numeric(
        series.astype(str).str.replace(",", "", regex=False)
                          .str.replace("%", "", regex=False)
                          .str.strip().replace({"N/A": None, "-": None, "": None}),
        errors="coerce")


def scrape_chain(symbol, expiry, headless=True):
    """Fusionne les deux vues et renvoie un DataFrame prêt pour cme_data."""
    driver = make_driver(headless)
    try:
        print("vue volatility-greeks (IV, gamma) :")
        greeks = scrape_view(driver, symbol, expiry, "volatility-greeks")
        print(f"   {len(greeks)} lignes")
        print("vue options (volume, open interest) :")
        chain = scrape_view(driver, symbol, expiry, "options")
        print(f"   {len(chain)} lignes")
    finally:
        driver.quit()

    for frame in (greeks, chain):
        frame["Strike"] = _num(frame["Strike"])
        frame["Type"] = frame["Type"].astype(str).str.strip().str.title().str[:4]

    keep_g = ["Strike", "Type"] + [c for c in ("Latest", "IV", "Delta", "Gamma") if c in greeks]
    keep_c = ["Strike", "Type"] + [c for c in ("Open Int", "Volume") if c in chain]
    merged = pd.merge(greeks[keep_g], chain[keep_c], on=["Strike", "Type"], how="outer")

    for col in merged.columns:
        if col != "Type":
            merged[col] = _num(merged[col])
    if "Open Int" in merged:
        merged = merged.rename(columns={"Open Int": "Open Interest"})
    return merged.dropna(subset=["Strike"]).sort_values(["Type", "Strike"])


def main():
    p = argparse.ArgumentParser(description="Scraper Barchart pour options sur futures")
    p.add_argument("symbol", nargs="?", default="E6U26",
                   help="contrat Barchart (E6U26 = Euro FX sept. 2026)")
    p.add_argument("--expiry", default="aug-26", help="échéance des options (aug-26, sep-26...)")
    p.add_argument("--out", default="barchart_options.csv", help="fichier CSV de sortie")
    p.add_argument("--visible", action="store_true", help="ouvrir un vrai navigateur")
    args = p.parse_args()

    try:
        df = scrape_chain(args.symbol, args.expiry, headless=not args.visible)
    except RuntimeError as err:
        raise SystemExit(f"\nEchec : {err}")

    df.to_csv(args.out, index=False)
    oi = df["Open Interest"].sum() if "Open Interest" in df else 0
    print(f"\n{len(df)} lignes -> {args.out}   (open interest total : {oi:,.0f})")
    if not oi:
        print("ATTENTION : open interest vide, le GEX ne pourra pas être calculé.")
    else:
        print(f"Ensuite : python main.py 6E --cme {args.out} --expiry AAAA-MM-JJ")
    return 0


if __name__ == "__main__":
    sys.exit(main())
