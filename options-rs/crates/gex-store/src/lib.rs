//! Lecture des relevés archivés.
//!
//! Le format n'est pas inventé ici : c'est celui que `snapshots.py` écrit depuis
//! le début du projet, colonnes au nom du CBOE comprises. Le respecter à la
//! lettre est ce qui permet à `--replay` de rouvrir une séance archivée il y a
//! des mois, et au moteur Rust de lire exactement ce que le collecteur Python
//! produit — sans quoi comparer les deux ne prouverait rien.
//!
//! **C'est ici, et seulement ici, que le projet touche à un fichier.**
//! `gex-core` ne déclare aucune dépendance capable de le faire.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ecriture;
pub mod historique;
pub mod validation;

use std::fs::File;
use std::path::Path;

use arrow_array::{Array, Float64Array, RecordBatch, TimestampMicrosecondArray};
use chrono::DateTime;
use gex_core::chaine::{Chaine, ChaineInvalide, Cote, Ligne};
use gex_core::temps::{EcheanceNy, InstantReleve};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Colonne portant le prix du sous-jacent, constante sur toutes les lignes.
const COLONNE_SPOT: &str = "_spot";
/// Colonne portant l'instant du relevé, constante elle aussi.
const COLONNE_DATE: &str = "_quote_date";
/// Colonne de l'échéance.
const COLONNE_ECHEANCE: &str = "ExpirationDate";
/// Colonne du strike.
const COLONNE_STRIKE: &str = "StrikePrice";

/// Ce qui empêche de lire un relevé.
#[derive(Debug)]
pub enum ErreurReleve {
    /// Le fichier est absent ou illisible.
    Fichier(std::io::Error),
    /// Le fichier n'est pas un parquet exploitable.
    Format(parquet::errors::ParquetError),
    /// Le parquet s'ouvre mais un lot de lignes ne se decode pas.
    ///
    /// Distinguee de `Format` : l'en-tete etait bon, c'est le contenu qui casse,
    /// et l'un se corrige en regenerant le fichier quand l'autre signale un
    /// fichier tronque.
    Decodage(arrow_schema::ArrowError),
    /// Une colonne indispensable manque.
    ///
    /// Distinguée du format : un parquet valide mais sans `_spot` n'est pas un
    /// relevé de ce projet, et le dire vaut mieux que rendre une chaîne vide.
    ColonneAbsente(&'static str),
    /// Une colonne existe mais n'a pas le type attendu.
    TypeInattendu(&'static str),
    /// Le relevé est syntaxiquement lisible mais ne décrit rien d'exploitable.
    Chaine(ChaineInvalide),
}

impl std::fmt::Display for ErreurReleve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurReleve::Fichier(e) => write!(f, "Relevé illisible : {e}"),
            ErreurReleve::Format(e) => write!(f, "Ce n'est pas un parquet exploitable : {e}"),
            ErreurReleve::Decodage(e) => write!(f, "Relevé tronqué ou corrompu : {e}"),
            ErreurReleve::ColonneAbsente(c) => write!(
                f,
                "Colonne {c:?} absente : ce fichier n'est pas un relevé de ce projet."
            ),
            ErreurReleve::TypeInattendu(c) => {
                write!(f, "Colonne {c:?} d'un type inattendu.")
            }
            ErreurReleve::Chaine(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErreurReleve {}

impl From<ChaineInvalide> for ErreurReleve {
    fn from(e: ChaineInvalide) -> Self {
        ErreurReleve::Chaine(e)
    }
}

/// Les colonnes d'un côté, dans l'ordre où [`Cote`] les attend.
const CHAMPS_CALL: [&str; 6] = [
    "CallIV",
    "CallGamma",
    "CallDelta",
    "CallOpenInt",
    "CallVega",
    "CallTheta",
];
const CHAMPS_PUT: [&str; 6] = [
    "PutIV",
    "PutGamma",
    "PutDelta",
    "PutOpenInt",
    "PutVega",
    "PutTheta",
];

/// Un vecteur de flottants, ou des zéros si la colonne n'existe pas.
///
/// Les colonnes de grecs sont facultatives par construction : le format les
/// crée à zéro quand la source ne les publie pas, et un relevé archivé avant
/// qu'elles n'existent doit rester rejouable. Zéro se lit comme « pas de
/// donnée » et ne contribue à aucune exposition.
fn colonne_ou_zeros(lot: &RecordBatch, nom: &str) -> Result<Vec<f64>, ErreurReleve> {
    let Some(colonne) = lot.column_by_name(nom) else {
        return Ok(vec![0.0; lot.num_rows()]);
    };
    let valeurs = colonne
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or(ErreurReleve::TypeInattendu("colonne numérique"))?;
    Ok((0..valeurs.len())
        .map(|i| if valeurs.is_null(i) { 0.0 } else { valeurs.value(i) })
        .collect())
}

/// Une colonne de flottants qui doit exister.
fn colonne_requise(lot: &RecordBatch, nom: &'static str) -> Result<Vec<f64>, ErreurReleve> {
    if lot.column_by_name(nom).is_none() {
        return Err(ErreurReleve::ColonneAbsente(nom));
    }
    colonne_ou_zeros(lot, nom)
}

/// Une colonne d'horodatages en microsecondes.
fn colonne_instants(
    lot: &RecordBatch,
    nom: &'static str,
) -> Result<Vec<chrono::NaiveDateTime>, ErreurReleve> {
    let colonne = lot
        .column_by_name(nom)
        .ok_or(ErreurReleve::ColonneAbsente(nom))?;
    let valeurs = colonne
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .ok_or(ErreurReleve::TypeInattendu(nom))?;
    (0..valeurs.len())
        .map(|i| {
            DateTime::from_timestamp_micros(valeurs.value(i))
                .map(|d| d.naive_utc())
                .ok_or(ErreurReleve::TypeInattendu(nom))
        })
        .collect()
}

/// Lit un relevé archivé et en fait une chaîne.
///
/// L'échéance y est entendue en heure de New York et l'instant du relevé en UTC :
/// c'est le contrat qu'écrit le collecteur, et le respecter est ce qui évite de
/// prendre un décalage de fuseau pour du temps restant.
pub fn lire_releve(chemin: impl AsRef<Path>) -> Result<Chaine, ErreurReleve> {
    let fichier = File::open(chemin.as_ref()).map_err(ErreurReleve::Fichier)?;
    let lecteur = ParquetRecordBatchReaderBuilder::try_new(fichier)
        .map_err(ErreurReleve::Format)?
        .build()
        .map_err(ErreurReleve::Format)?;

    let mut lignes: Vec<Ligne> = Vec::new();
    let mut spot: Option<f64> = None;
    let mut releve: Option<chrono::NaiveDateTime> = None;

    for lot in lecteur {
        let lot = lot.map_err(ErreurReleve::Decodage)?;
        if lot.num_rows() == 0 {
            continue;
        }
        let strikes = colonne_requise(&lot, COLONNE_STRIKE)?;
        let echeances = colonne_instants(&lot, COLONNE_ECHEANCE)?;
        let spots = colonne_requise(&lot, COLONNE_SPOT)?;
        let dates = colonne_instants(&lot, COLONNE_DATE)?;

        // Constantes par construction : on prend la première et on n'y revient
        // pas. Un relevé qui les ferait varier ne serait pas un relevé.
        spot.get_or_insert(spots[0]);
        releve.get_or_insert(dates[0]);

        let call: Vec<Vec<f64>> = CHAMPS_CALL
            .iter()
            .map(|nom| colonne_ou_zeros(&lot, nom))
            .collect::<Result<_, _>>()?;
        let put: Vec<Vec<f64>> = CHAMPS_PUT
            .iter()
            .map(|nom| colonne_ou_zeros(&lot, nom))
            .collect::<Result<_, _>>()?;

        for i in 0..lot.num_rows() {
            lignes.push(Ligne {
                echeance: EcheanceNy(echeances[i]),
                strike: strikes[i],
                call: Cote {
                    iv: call[0][i],
                    gamma: call[1][i],
                    delta: call[2][i],
                    open_interest: call[3][i],
                    vega: call[4][i],
                    theta: call[5][i],
                },
                put: Cote {
                    iv: put[0][i],
                    gamma: put[1][i],
                    delta: put[2][i],
                    open_interest: put[3][i],
                    vega: put[4][i],
                    theta: put[5][i],
                },
            });
        }
    }

    let spot = spot.ok_or(ErreurReleve::ColonneAbsente(COLONNE_SPOT))?;
    let releve = releve.ok_or(ErreurReleve::ColonneAbsente(COLONNE_DATE))?;
    Ok(Chaine::nouvelle(lignes, spot, InstantReleve(releve))?)
}

/// Le chemin du relevé courant d'un produit, celui que le collecteur réécrit.
///
/// Chemin fixe et non horodaté : un collecteur qui réécrit toutes les quinze
/// secondes créerait sinon près de mille cinq cents fichiers par jour, et le
/// lecteur ne saurait lequel est le dernier sans lister le dossier.
pub fn chemin_courant(dossier: impl AsRef<Path>, produit: &str) -> std::path::PathBuf {
    dossier
        .as_ref()
        .join(produit.to_ascii_uppercase())
        .join("courant.parquet")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/nq-2026-08-25.parquet")
    }

    /// Le relevé IB réel du 25 août 2026, collecté en différé depuis TWS.
    #[test]
    fn lit_le_releve_ib_reel() {
        let c = lire_releve(fixture()).expect("le relevé doit se lire");
        assert_eq!(c.lignes().len(), 568);
        assert!((c.spot - 29_305.75).abs() < 1e-9, "spot {}", c.spot);
        assert_eq!(
            c.releve.0.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-08-25 20:30:16"
        );
    }

    /// Les échéances portent leur heure de règlement, pas minuit. C'est la
    /// correction qui a retiré un artefact de dix-neuf pour cent au GEX.
    #[test]
    fn les_echeances_portent_leur_heure_de_reglement() {
        let c = lire_releve(fixture()).unwrap();
        let echeances = c.echeances();
        assert_eq!(echeances.len(), 4);
        for e in &echeances {
            assert_eq!(
                e.0.format("%H:%M").to_string(),
                "16:00",
                "échéance {} sans heure de règlement",
                e.0
            );
        }
    }

    #[test]
    fn les_open_interest_et_les_iv_sont_servis() {
        let c = lire_releve(fixture()).unwrap();
        let avec_oi = c
            .lignes()
            .iter()
            .filter(|l| l.call.open_interest + l.put.open_interest > 0.0)
            .count();
        let avec_iv = c.lignes().iter().filter(|l| l.call.iv > 0.0).count();
        assert!(avec_oi > 0, "aucun open interest lu");
        assert!(avec_iv > 0, "aucune volatilité implicite lue");
    }

    #[test]
    fn un_fichier_absent_le_dit() {
        let e = lire_releve("nulle-part/courant.parquet").unwrap_err();
        assert!(matches!(e, ErreurReleve::Fichier(_)), "{e}");
    }

    #[test]
    fn le_chemin_courant_est_fixe() {
        let p = chemin_courant("snapshots", "nq");
        assert!(p.ends_with("NQ/courant.parquet") || p.ends_with("NQ\\courant.parquet"), "{p:?}");
    }
}
