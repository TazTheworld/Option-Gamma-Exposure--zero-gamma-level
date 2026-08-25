# Conventions du dépôt

Gamma exposure et zero gamma level à partir de chaînes d'options. Ce fichier
consigne les règles que le code suit déjà — les enfreindre casse la cohérence
avant de casser les tests.

## Environnement

```sh
venv\Scripts\activate                 # Windows
python -m venv venv                   # première fois
pip install -r requirements.txt
pip install -e ".[dev]"               # pytest, pyarrow
python -m pytest tests -q             # doit passer sans réseau
```

**Ne jamais régénérer `requirements.txt` avec `pip freeze`.** Il ne liste que le
cœur — numpy, pandas, scipy, matplotlib, requests — pour que `python main.py
TSLA` n'impose ni selenium, ni databento, ni pyarrow. Tout le reste est un extra
déclaré dans `pyproject.toml` : `.[databento]`, `.[cme]`, `.[barchart]`,
`.[snapshots]`, `.[ib]`, `.[dev]`.

Un nouveau module qui a besoin d'une dépendance lourde déclare un extra et
l'importe **dans la fonction qui s'en sert**, jamais en tête de fichier.

## Langue

Français pour le métier, anglais pour les conventions externes. Le partage est
net et déjà appliqué partout :

| Français | Anglais |
|---|---|
| `analyser`, `murs`, `expositions`, `croisements_zero` | `fetch_chain`, `build_chain`, `load_from_csv` |
| `charger`, `sauver`, `lister`, `dernier` | `black76_gamma`, `implied_vol` |
| `perimetre`, `fusionner`, `selection_vif` | `COLUMNS`, `StrikePrice`, `OpenInt` |

Les docstrings et les commentaires sont en français, **avec les accents**. Les
noms de colonnes suivent le format CBOE et restent en anglais.

## Ce que les commentaires doivent dire

Le dépôt n'explique pas ce que fait le code — il explique **ce qui se passerait
sans lui**. C'est la règle la plus visible à la lecture, et la plus utile :

> « Le tri secondaire n'est pas cosmétique : à poids égaux, sans lui, deux appels
> rendraient deux listes différentes et le recyclage annulerait puis
> re-souscrirait les mêmes contrats pour rien. »

Un commentaire qui paraphrase la ligne suivante n'a pas sa place.

## Architecture

**Tout ce qui produit un chiffre est testable sans réseau.** C'est la contrainte
qui structure le reste.

| Module | Rôle |
|---|---|
| `greeks.py` | **le seul endroit** où une formule Black-Scholes est écrite |
| `cme_data.py` | Black-76 pour les options sur futures |
| `analysis.py` | filtre d'échéance, expositions, murs, profil, zero gamma |
| `main.py` | interface en ligne de commande, rien d'autre |

Une formule ne s'écrit pas deux fois. Si un module en a besoin, il l'importe.

### Le patron des sources de données

Toute source suit la même forme, et `databento_data.py` la documente :

- une **fonction pure d'assemblage** (`build_chain`) qui prend des données déjà
  téléchargées et rend `(df, spot, quote_date)` ;
- une **couche réseau mince** qui ne fait que chercher ces données.

La fonction pure est testée sur des trames fabriquées ; la couche réseau ne l'est
pas. Deux assembleurs qui divergeraient sur un détail — la normalisation de l'IV,
par exemple — donneraient deux GEX différents pour la même chaîne.

### Le format pivot

Toute chaîne produite respecte `cboe_data.COLUMNS` + `COLONNES_GRECS`, et sort de
`cboe_data._clean()`. En aval, `analysis.analyser(df, spot, quote_date, …)` ne
sait pas d'où vient la chaîne, et ne doit pas avoir à le savoir.

## Tests

**Aucun test ne touche au réseau.** La CI l'exige : « la suite doit passer telle
quelle ». Un test qui aurait besoin d'un appel réseau teste la mauvaise chose.

Les jeux d'essai sont **construits depuis des paramètres connus** — spot, IV,
open interest placés à des strikes choisis — pour vérifier qu'on retrouve la
vérité terrain. On ne fige jamais une sortie observée : un tel test passe encore
quand le calcul devient faux.

Un test qui dépend d'un extra se saute proprement plutôt que d'échouer, sur le
modèle de `besoin_databento` dans `tests/test_sources.py`. La CI, elle, installe
l'extra : sauter partout reviendrait à ne rien tester.

## Ne jamais faire en silence

Principe explicite, énoncé dans `price_data.py` à propos des indices substitués
par un ETF : **annoncé à l'écran, jamais fait en silence.** S'applique à toute
substitution, tout mode dégradé, toute approximation — données différées, gamma
recalculé faute d'être publié, contexte de séance absent.

Et échouer bruyamment vaut mieux que rendre du vide : une chaîne vide donnerait
un GEX de zéro, qui est un chiffre et non une erreur.

## Git

Identité du dépôt :

```sh
git config user.name "Taz"
git config user.email "82410649+TazTheworld@users.noreply.github.com"
```

Messages de commit **en français sans accents** : un titre court à l'infinitif ou
au constat, puis un corps en prose qui explique **pourquoi** et ce que le
changement corrige. Pas de liste à puces, pas de résumé du diff — le diff est
déjà là. Terminer par le nombre de tests quand il change.

Le travail se fait directement sur `main`.

## Documents de conception

- Spécifications : `docs/superpowers/specs/AAAA-MM-JJ-<sujet>-design.md`
- Plans d'implémentation : `docs/superpowers/plans/AAAA-MM-JJ-<sujet>.md`

Ils ne décrivent pas seulement ce qui est retenu, mais **ce qui a été écarté et
pourquoi** — c'est ce qui évite de refaire deux fois le même arbitrage.

## Chantier en cours

Collecteur Interactive Brokers pour les options sur futures NQ.

- Conception : `docs/superpowers/specs/2026-08-25-collecteur-ib-nq-design.md`
- Plan du lot hors ligne : `docs/superpowers/plans/2026-08-25-collecteur-ib-nq-hors-ligne.md`

**Fait :** les cinq fonctions pures d'`ib_data.py` (`echeances_utiles`,
`perimetre`, `build_chain`, `selection_vif`, `fusionner`), `snapshots.courant()`,
et `main.py --suivre`.

**Bloqué sur une vérification manuelle :** l'open interest par contrat sur les
options sur futures. Ouvrir une chaîne NQ dans TWS et ajouter la colonne *Open
Interest* — l'API ne dépêche que ce que TWS affiche déjà. Si la colonne reste
vide, le socle bascule sur le fichier End-of-Day gratuit du CME, que
`cme_data.load_settlement()` lit déjà ; les cinq fonctions restent valables dans
les deux cas.

**Reporté, décidé :** le nettoyage de `databento_data.py` attend que le collecteur
IB tourne — supprimer le chemin actuel avant que son remplaçant existe laisserait
les options sur futures sans accès automatisé.
