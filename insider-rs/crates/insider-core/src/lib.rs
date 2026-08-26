//! Le vocabulaire commun des déclarations d'initiés.
//!
//! Trois sources, trois formats, un seul jeu de types : les formulaires 4 de la
//! SEC pour les dirigeants d'entreprise, les PDF du greffe de la Chambre et le
//! HTML du Sénat pour les élus. Ce qu'elles décrivent est la même chose — qui a
//! échangé quoi, quand, et pour combien — et le faire dire par les mêmes types
//! est ce qui permet de les concaténer sans retouche.
//!
//! **Cette crate ne sait rien lire ni écrire.** Elle ne déclare aucune
//! dépendance capable d'ouvrir un fichier ou un socket, et `chrono` y est
//! déclaré sans sa feature `clock` : une date se lit dans un document, jamais à
//! l'horloge. Un parseur qui daterait une transaction du jour où on le lance ne
//! compile pas.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod montant;
pub mod transaction;

pub use montant::Montant;
pub use transaction::{Detenteur, Nature, Operation, Sens, Source};
