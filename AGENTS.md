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
pip install -e ".[ib]"                # ib_async, pour le collecteur seulement
python -m pytest tests -q             # doit passer sans réseau

cargo test --manifest-path options-rs/Cargo.toml
cargo clippy --manifest-path options-rs/Cargo.toml --all-targets -- -D warnings
```

**Ne jamais régénérer `requirements.txt` avec `pip freeze`.** Il ne sert plus qu'à
`sec_data.py` : pandas et requests, rien d'autre. Le moteur d'options n'a plus
aucune dépendance Python.

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

**Le dépôt est en Rust**, dans `options-rs/` — voir son README, qui porte les
conventions du moteur. Le seul Python restant est `sec_data.py`, la lecture des
déclarations d'initiés SEC, qui n'a jamais eu de rapport avec les options et
attend son propre dossier.

| Crate | Rôle |
|---|---|
| `gex-core` | **aucune E/S possible** : Black-76, greeks, expositions, murs, profil |
| `gex-ib` | la source : décisions de collecte, et la couche réseau TWS |
| `gex-store` | relevés parquet, historique, validation |
| `gex-collector` | le démon |
| `gex-cli` | le lecteur |

Les graphiques ont été abandonnés au passage, volontairement : `plotters` aurait
coûté plus cher que ce qu'il rapportait.

Une formule ne s'écrit pas deux fois. Si un module en a besoin, il l'importe.

### Le patron des sources de données

Il n'y a plus qu'une source — le CBOE, le CME, Databento et Barchart ont été
retirés — mais toute source qu'on rajouterait suit cette forme, et `gex-ib` la
documente en la découpant en deux modules :

- une **fonction pure d'assemblage** (`build_chain`) qui prend des données déjà
  téléchargées et rend `(df, spot, quote_date)` ;
- une **couche réseau mince** qui ne fait que chercher ces données.

La fonction pure est testée sur des trames fabriquées ; la couche réseau ne l'est
pas. Deux assembleurs qui divergeraient sur un détail — la normalisation de l'IV,
par exemple — donneraient deux GEX différents pour la même chaîne.

### Le format pivot

Toute chaîne produite est une `gex_core::chaine::Chaine`. En aval, `analyser()` ne
sait pas d'où elle vient, et ne doit pas avoir à le savoir.

Les colonnes du fichier parquet gardent leurs noms hérités du CBOE — `CallOpenInt`,
`PutIV`, `StrikePrice`. Ils ne sont plus la vérité du calcul, mais restent celle du
fichier : les renommer rendrait illisibles des relevés que le rejeu doit encore
pouvoir ouvrir.

## Tests

**Aucun test ne touche au réseau.** Ni à TWS. La suite doit passer sur une machine
qui n'a ni passerelle ni connexion — un test qui aurait besoin d'un appel réseau
teste la mauvaise chose. Il n'y a pas d'intégration continue pour le rappeler :
c'est à la main, avant de pousser.

Les jeux d'essai sont **construits depuis des paramètres connus** — spot, IV,
open interest placés à des strikes choisis — pour vérifier qu'on retrouve la
vérité terrain. On ne fige jamais une sortie observée : un tel test passe encore
quand le calcul devient faux.

Aucun test n'importe `ib_async` : les fonctions pures d'`ib_data.py` se vérifient
hors ligne, et c'est ce qui permet à la suite de couvrir le calcul sans TWS. La CI
installe quand même l'extra, parce que c'est la seule chose qui vérifie qu'il se
résout.

**Le multiplicateur est le piège maison.** Se tromper de contrat ne produit aucune
erreur : un GEX cinq fois trop grand reste un nombre plausible. `black76.py` refuse
donc de deviner, et `tests/test_main.py` ferme la régression.

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
