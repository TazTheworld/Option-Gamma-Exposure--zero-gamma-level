# ibapi 3.3.0, corrigé d'une ligne

Copie de la crate publiée, avec **une seule modification** — et la raison vaut
d'être écrite, parce que le symptôme ne ressemblait pas du tout à sa cause.

## La ligne

`src/messages.rs` :

```diff
-pub const DATA_ADVISORY_CODES: [i32; 2] = [10089, 10167];
+pub const DATA_ADVISORY_CODES: [i32; 4] = [10089, 10090, 10091, 10167];
```

Les tests et exemples de la crate ont été retirés pour alléger — c'est tout.

## Ce que ça corrige

`ibapi` classe chaque message d'erreur d'IB en *avertissement* ou en *erreur*.
Un avertissement est routé vers l'abonnement et **le flux reste ouvert** ; une
erreur le **ferme définitivement** (`stream_ended`).

Le code **10090** — « il vous manque l'abonnement pour une partie des données
demandées ; des données de marché en différé sont disponibles » — tombait du
mauvais côté. Or c'est précisément un avis de repli : IB continue d'émettre, en
différé. Le flux mourait donc avant les données qu'il annonçait servir.

La liste `DATA_ADVISORY_CODES` existe pour exactement ce cas, et sa propre
documentation le dit :

> *the request proceeded with a fallback (delayed market data) rather than
> failing. Informational.*

10089 et 10167 y figuraient, 10090 et 10091 non. C'est un oubli, pas un choix.

## Comment le défaut s'est manifesté

En collecte, chaque souscription d'option rendait **un** message puis se taisait.
Le collecteur voyait des contrats muets, indiscernables de contrats sans open
interest — donc un GEX de zéro, sans erreur.

Le même contrat, à la même seconde, lu par `ib_async` en Python :

```
IV 0.242 | gamma 0.000936 | undPrice 29217.5 | callOpenInt 2.0 | bid/ask 167.5/171.0
```

`ib_async` journalise le 10090 et continue de lire. C'est cette comparaison qui a
désigné le client Rust plutôt que le compte IB, après plusieurs fausses pistes —
quota de lignes saturé, abonnement manquant, marché fermé.

Après correctif, sur le même contrat :

```
essai 1 : 28 données, 0 refus
essai 2 : 32 données, 0 refus
essai 3 : 30 données, 0 refus
```

## Pourquoi une copie plutôt qu'un fork

Une copie garde le dépôt autonome : `cargo build` suffit, sans dépôt tiers ni
accès réseau supplémentaire. Le prix est 2,7 Mo de source figée à la version
3.3.0.

Le correctif mérite d'être proposé en amont — <https://github.com/wboayue/rust-ibapi>.
Le jour où il y est intégré, cette copie et la section `[patch.crates-io]` du
manifeste disparaissent ensemble.
