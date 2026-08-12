"""Le GEX tient-il ses promesses ? Test sur l'historique accumulé.

Le modèle avance deux affirmations vérifiables :

  1. en gamma négatif, les mouvements sont plus amples qu'en gamma positif ;
  2. le prix bute sur le call wall et se soutient sur le put wall.

Ce script les mesure sur history.csv, sans source de prix externe : la colonne
`spot` de l'historique constitue la série. Il faut donc des relevés réguliers —
un par séance idéalement.

    python validate.py            # tous les tickers
    python validate.py SPCX

Ce n'est pas un backtest de stratégie : on mesure si la description du terrain
est exacte, pas si on peut en tirer de l'argent.
"""

import argparse

import numpy as np
import pandas as pd

import history

# En dessous, aucune conclusion n'est défendable — on affiche mais on le dit.
N_MINIMAL = 20


def prepare(df):
    """Ajoute le mouvement observé jusqu'au relevé suivant."""
    df = df.sort_values("timestamp").copy()
    df["date"] = pd.to_datetime(df.timestamp)
    df["spot_suivant"] = df.spot.shift(-1)
    df["jours"] = (df.date.shift(-1) - df.date).dt.total_seconds() / 86400
    df = df[(df.jours > 0) & df.spot_suivant.notna()].copy()
    df["rendement"] = (df.spot_suivant - df.spot) / df.spot
    # Ramené à une base journalière pour comparer des intervalles inégaux
    df["mouvement_par_jour"] = df.rendement.abs() / np.sqrt(df.jours)
    return df


def test_regime(df):
    """Affirmation 1 : |mouvement| plus grand quand le gamma est négatif."""
    neg = df[df.total_gex < 0].mouvement_par_jour
    pos = df[df.total_gex > 0].mouvement_par_jour
    print("\n1. AMPLITUDE SELON LE RÉGIME")
    print(f"   gamma négatif : {len(neg):>3} relevés, mouvement médian "
          f"{neg.median()*100:.2f}% / jour" if len(neg) else
          "   gamma négatif :   0 relevé")
    print(f"   gamma positif : {len(pos):>3} relevés, mouvement médian "
          f"{pos.median()*100:.2f}% / jour" if len(pos) else
          "   gamma positif :   0 relevé")

    if len(neg) < 3 or len(pos) < 3:
        print("   -> trop peu de relevés dans l'un des deux régimes pour comparer")
        return None
    ecart = neg.median() / pos.median() - 1
    print(f"   -> les mouvements sont {abs(ecart)*100:.0f}% plus "
          f"{'amples' if ecart > 0 else 'faibles'} en gamma négatif")
    if ecart > 0:
        print("      conforme au modèle" if len(df) >= N_MINIMAL else
              "      conforme au modèle, mais l'échantillon est trop court pour conclure")
    else:
        print("      CONTRAIRE au modèle" if len(df) >= N_MINIMAL else
              "      contraire au modèle, mais l'échantillon est trop court pour conclure")
    return ecart


def test_zero_gamma(df):
    """Affirmation 1 bis : c'est la position vis-à-vis du zero gamma qui compte."""
    d = df.dropna(subset=["zero_gamma"])
    sous = d[d.spot < d.zero_gamma].mouvement_par_jour
    sur = d[d.spot >= d.zero_gamma].mouvement_par_jour
    print("\n2. POSITION VIS-À-VIS DU ZERO GAMMA")
    print(f"   spot sous le zero gamma : {len(sous):>3} relevés"
          + (f", médiane {sous.median()*100:.2f}% / jour" if len(sous) else ""))
    print(f"   spot au-dessus          : {len(sur):>3} relevés"
          + (f", médiane {sur.median()*100:.2f}% / jour" if len(sur) else ""))
    if len(sous) >= 3 and len(sur) >= 3:
        e = sous.median() / sur.median() - 1
        print(f"   -> {abs(e)*100:.0f}% plus {'ample' if e > 0 else 'calme'} sous le zero gamma")
        return e
    print("   -> trop peu de relevés de part et d'autre")
    return None


def test_murs(df):
    """Affirmation 2 : le prix franchit-il les murs, ou butent-ils ?"""
    print("\n3. LES MURS SONT-ILS RESPECTÉS ?")
    for col, nom, sens in (("call_wall", "call wall", "au-dessus"),
                           ("put_wall", "put wall", "en dessous")):
        d = df.dropna(subset=[col])
        d = d[(d[col] > 0)]
        if d.empty:
            print(f"   {nom:<10} : aucun relevé")
            continue
        if sens == "au-dessus":
            distance = (d[col] - d.spot) / d.spot
            franchi = d.spot_suivant > d[col]
        else:
            distance = (d.spot - d[col]) / d.spot
            franchi = d.spot_suivant < d[col]
        approche = d[distance.between(0, 0.05)]   # mur à moins de 5 %
        print(f"   {nom:<10} : {len(d):>3} relevés, franchi {franchi.mean()*100:.0f}% du temps"
              f"   (distance médiane {distance.median()*100:+.1f}%)")
        if len(approche) >= 3:
            f2 = franchi[approche.index]
            print(f"              dont {len(approche)} à moins de 5 % : "
                  f"franchi {f2.mean()*100:.0f}% du temps")


def valider(path=history.DEFAUT, ticker=None):
    brut = history.load(path, ticker)
    if brut.empty:
        print("historique vide")
        return

    for tk, groupe in brut.groupby("ticker"):
        df = prepare(groupe)
        print(f"\n{'='*62}\n{tk} — {len(groupe)} relevés, {len(df)} intervalles exploitables")
        if len(df) < 2:
            print("  pas assez de relevés consécutifs. Lance main.py régulièrement :\n"
                  "  la colonne spot de l'historique sert de série de prix.")
            continue
        print(f"  du {df.date.min():%Y-%m-%d} au {df.date.max():%Y-%m-%d}"
              f" | intervalle médian {df.jours.median():.1f} jour(s)")
        if len(df) < N_MINIMAL:
            print(f"  ATTENTION : {len(df)} intervalles, il en faudrait au moins {N_MINIMAL}.")
            print("  Les chiffres ci-dessous sont indicatifs, pas concluants.")
        test_regime(df)
        test_zero_gamma(df)
        test_murs(df)


def main():
    p = argparse.ArgumentParser(description="Valider les affirmations du modèle GEX")
    p.add_argument("ticker", nargs="?", help="sous-jacent ; omis, teste tous")
    p.add_argument("--file", default=history.DEFAUT, help="fichier d'historique")
    args = p.parse_args()
    valider(args.file, args.ticker)


if __name__ == "__main__":
    try:
        main()
    except (FileNotFoundError, ValueError) as err:
        raise SystemExit(f"Erreur : {err}")
