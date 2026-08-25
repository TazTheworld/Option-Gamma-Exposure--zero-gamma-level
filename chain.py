"""Le format pivot des chaînes d'options : les colonnes, et leur nettoyage.

Ces définitions vivaient dans `cboe_data.py`, du temps où le CBOE était la source
de référence et où les autres s'alignaient sur elle. La source est partie, le
format reste : `analysis.py` le lit, `plots.py` en trace les colonnes,
`snapshots.py` l'archive, `ib_data.py` le produit. Le loger dans un module qui
n'est le fournisseur de personne évite qu'il redevienne l'annexe d'une source, et
qu'en supprimer une emporte le format avec elle.

Les noms de colonnes restent en anglais et au format CBOE : ils sont écrits dans
les relevés déjà archivés, et les renommer rendrait illisibles des fichiers que
`--replay` doit encore pouvoir ouvrir.
"""

import pandas as pd

COLUMNS = [
    "ExpirationDate", "Calls", "CallLastSale", "CallNet", "CallBid", "CallAsk",
    "CallVol", "CallIV", "CallDelta", "CallGamma", "CallOpenInt", "StrikePrice",
    "Puts", "PutLastSale", "PutNet", "PutBid", "PutAsk", "PutVol", "PutIV",
    "PutDelta", "PutGamma", "PutOpenInt",
]

# Grecs ajoutés APRÈS COLUMNS et jamais dedans : les relevés archivés affectent
# les colonnes une à une dans l'ordre, et allonger COLUMNS décalerait tout le
# tableau d'une séance rejouée.
COLONNES_GRECS = ["CallVega", "PutVega", "CallTheta", "PutTheta"]


def _clean(df):
    """Typage numérique + suppression des lignes inexploitables.

    Les grecs optionnels sont créés à zéro quand la source ne les publie pas :
    les expositions correspondantes valent alors zéro plutôt que d'être absentes,
    et rien en aval n'a à tester leur présence.
    """
    for colonne in COLONNES_GRECS:
        if colonne not in df.columns:
            df[colonne] = 0.0
    numeric = ["StrikePrice", "CallIV", "PutIV", "CallGamma", "PutGamma",
               "CallOpenInt", "PutOpenInt", "CallDelta", "PutDelta", *COLONNES_GRECS]
    for col in numeric:
        df[col] = pd.to_numeric(df[col], errors="coerce")

    # Un strike sans OI ni des deux côtés n'apporte rien au calcul de GEX
    df[["CallOpenInt", "PutOpenInt"]] = df[["CallOpenInt", "PutOpenInt"]].fillna(0.0)
    df[["CallGamma", "PutGamma"]] = df[["CallGamma", "PutGamma"]].fillna(0.0)
    df[["CallDelta", "PutDelta"]] = df[["CallDelta", "PutDelta"]].fillna(0.0)
    df[COLONNES_GRECS] = df[COLONNES_GRECS].fillna(0.0)
    df[["CallIV", "PutIV"]] = df[["CallIV", "PutIV"]].fillna(0.0)
    df = df.dropna(subset=["StrikePrice", "ExpirationDate"])
    return df.sort_values(["ExpirationDate", "StrikePrice"]).reset_index(drop=True)
