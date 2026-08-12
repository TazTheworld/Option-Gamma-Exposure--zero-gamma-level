"""Le GEX tient-il ses promesses ? Test sur l'historique accumulé.

Le modèle avance deux affirmations vérifiables :

  1. en gamma négatif, les mouvements sont plus amples qu'en gamma positif ;
  2. le prix bute sur le call wall et se soutient sur le put wall.

Ce script les mesure sur history.csv, sans source de prix externe : la colonne
`spot` de l'historique constitue la série. Il faut donc des relevés réguliers —
un par séance idéalement.

La mesure est faite séparément pour chaque (sous-jacent, dte_max, source de gamma).
Mélanger des périmètres reviendrait à classer le régime d'après un GEX qui change
de signe rien qu'en changeant d'horizon. Une seule séance compte une fois : deux
exécutions du même jour ne sont pas deux observations.

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

# Deux exécutions rapprochées ne forment pas une observation : normalisé en
# racine du temps, un mouvement réel de 0,2 % sur 20 minutes ressort à 1,7 %
# par jour, et cette valeur entre telle quelle dans la médiane comparée plus bas.
# On garde donc une seule ligne par séance, et on écarte ce qui reste trop court.
INTERVALLE_MINIMAL = 0.5      # en jours


def dedoublonner(df):
    """Une ligne par séance : la dernière. Les relevés intrajournaliers ne sont
    pas des observations indépendantes, seulement la même séance revue."""
    df = df.copy()
    df["date"] = pd.to_datetime(df.timestamp)
    df["seance"] = df.date.dt.normalize()
    return (df.sort_values("date")
              .drop_duplicates(subset=["seance"], keep="last")
              .drop(columns=["seance"])
              .reset_index(drop=True))


def prepare(df, intervalle_minimal=INTERVALLE_MINIMAL):
    """Ajoute le mouvement observé jusqu'au relevé suivant.

    À n'appeler que sur un périmètre homogène (même ticker, même dte_max, même
    source de gamma) : voir valider().
    """
    df = dedoublonner(df)
    df["spot_suivant"] = df.spot.shift(-1)
    df["jours"] = (df.date.shift(-1) - df.date).dt.total_seconds() / 86400
    df = df[(df.jours >= intervalle_minimal) & df.spot_suivant.notna()].copy()
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
    """Mesure les affirmations du modèle, un périmètre à la fois.

    La segmentation par (ticker, dte_max, source de gamma) n'est pas cosmétique :
    le signe du GEX dépend de l'horizon retenu — le README en donne l'exemple, une
    même séance du SPX à +70,8 Md sur toute la chaîne et -1,1 Md sur le 0-7 DTE.
    Enchaîner les deux dans une même série classait le régime d'après un chiffre
    qui changeait de signe rien qu'en changeant d'horizon.
    """
    brut = history.load(path, ticker)
    if brut.empty:
        print("historique vide")
        return

    for cle, groupe in brut.groupby(history.CLES, dropna=False, sort=True):
        tk, dte, source, conv = cle
        # Un relevé écrit avant l'ajout d'une colonne n'a pas la valeur
        dte, source, conv = ("?" if pd.isna(v) else v for v in (dte, source, conv))
        df = prepare(groupe)
        seances = len(dedoublonner(groupe))
        print(f"\n{'='*62}\n{tk} (dte_max={dte}, gamma={source}, T={conv}) — "
              f"{len(groupe)} relevés, {seances} séances, "
              f"{len(df)} intervalles exploitables")
        if len(groupe) > seances:
            print(f"  {len(groupe) - seances} relevé(s) intrajournalier(s) écarté(s) : "
                  "une séance ne compte qu'une fois.")
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
