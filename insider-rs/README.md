# insider-rs — les déclarations d'initiés

Qui achète ce qu'il connaît. Deux familles d'initiés, trois sources, un seul jeu
de types.

| Source | Qui | Délai légal | Format |
|---|---|---|---|
| SEC, formulaire 4 | dirigeants, administrateurs, détenteurs > 10 % | 2 jours ouvrés | XML |
| Chambre, STOCK Act | représentants | 45 jours | PDF |
| Sénat, STOCK Act | sénateurs | 45 jours | HTML |

## La découpe

```
insider-rs/
  crates/
    insider-core/     le vocabulaire : opération, montant, nature   ← AUCUNE E/S
    insider-sec/      formulaires 4                                  (à venir)
    insider-house/    l'index annuel et les PDF du greffe            (à venir)
    insider-senate/   la recherche paginée et le HTML                (à venir)
    insider-store/    cache disque et export                         (à venir)
    insider-cli/      le binaire                                     (à venir)
  fixtures/           les oracles : des documents réels et ce que Python en tire
```

Même principe que `options-rs`, et pour la même raison : **`insider-core` ne
déclare aucune dépendance capable d'ouvrir un fichier ou un socket**, et `chrono`
y est déclaré sans sa feature `clock`. Une date se lit dans un document, jamais à
l'horloge — un parseur qui daterait une transaction du jour où on le lance ne
compile pas.

## Ce que les types empêchent d'écrire

**Un montant n'est pas un nombre.** Les déclarations donnent des *fourchettes* —
`$1,001 - $15,000` — et jamais la valeur exacte. Un `f64` inviterait à sommer, or
sommer des planchers sous-estime autant que sommer des plafonds surestime. D'où
`Montant`, avec une variante distincte pour `Over $50,000,000` : le plafond n'y
est pas une donnée manquante qu'on pourrait combler, il **n'existe pas**.

**Une acquisition n'est pas une décision.** La majorité des déclarations ne sont
pas des choix d'investissement : une attribution est une rémunération, un exercice
suivi d'une retenue fiscale est mécanique, une vente est souvent programmée des
mois à l'avance. `Sens` et `Nature` sont donc deux axes indépendants — une
attribution augmente la position sans engager personne.

**Une date de notification absente n'est pas un délai nul.** Le Sénat ne publie
pas cette date ; la recopier depuis celle de l'opération ferait croire à une
déclaration immédiate. `delai_notification()` rend `None`, pas zéro.

## L'état

| | Python | Rust |
|---|---|---|
| montants et fourchettes | les trois modules | ✅ `montant.rs` |
| vocabulaire commun | (implicite, par colonnes) | ✅ `transaction.rs` |
| formulaires 4 | `sec_data.py` | ⬜ `insider-sec` |
| Chambre | `congres_data.py` | ⬜ `insider-house` |
| Sénat | `senat_data.py` | ⬜ `insider-senate` |
| cache et export | ⬜ | ⬜ `insider-store` |

Le Python reste l'autorité tant que le portage n'est pas complet, et sert
d'oracle : ce que Rust produit doit correspondre, faute de quoi l'un des deux se
trompe.

## Le PDF, vérifié avant de s'engager

Les rapports de la Chambre sont des PDF chiffrés à mot de passe vide.
`pdf-extract 0.9` échoue dessus (`IncorrectPassword`) ; **la 0.12 les lit**, avec
exactement les mêmes artefacts qu'en Python — les libellés y sont espacés
caractère par caractère par des octets nuls, `Filing Status` devenant
`F\0\0\0\0 S\0\0\0\0`.

C'était le point qui décidait de la faisabilité, d'où la vérification avant la
première ligne de portage.

## Tests

```sh
cd insider-rs
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Aucun test ne touche au réseau. Ce qui ne se vérifie qu'en ligne vit dans
`examples/`, comme la sonde TWS d'`options-rs`.

Les conventions du dépôt s'appliquent : voir `AGENTS.md` à la racine.
