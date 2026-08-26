//! Écriture des relevés : le courant, et les archives horodatées.
//!
//! Le format n'est pas choisi ici — il est imposé par ce que `lire_releve` sait
//! rouvrir, et par les relevés déjà archivés. Les colonnes portent les noms du
//! CBOE, hérités du temps où c'était la source de référence : les renommer
//! rendrait illisibles des fichiers que le rejeu doit encore pouvoir ouvrir.
//!
//! Deux fichiers, deux cadences, et la raison tient au volume. Le **courant** est
//! réécrit en place toutes les quinze secondes ; l'horodater créerait près de
//! mille cinq cents fichiers par jour et par sous-jacent, et le lecteur ne saurait
//! lequel est le dernier sans lister le dossier. L'**archive** est horodatée
//! toutes les quinze minutes, et c'est elle qui alimente le rejeu.

use std::fs::{File, create_dir_all};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Float64Array, RecordBatch, StringArray, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use gex_core::chaine::Chaine;
use parquet::arrow::ArrowWriter;

use crate::{ErreurReleve, chemin_courant};

/// Les colonnes du format pivot, dans l'ordre où elles sont écrites.
///
/// Beaucoup restent à zéro : ni le carnet ni le dernier prix ne servent au
/// calcul. Elles sont écrites quand même parce que le format est le contrat entre
/// le collecteur et tout ce qui relira ces fichiers — y compris dans six mois, et
/// y compris par un outil qui n'existe pas encore.
const COLONNES_TEXTE: [&str; 2] = ["Calls", "Puts"];

/// Les colonnes numériques, dans l'ordre. Le premier élément de chaque paire est
/// le nom écrit, le second dit comment le remplir.
const COLONNES_NUM: [&str; 24] = [
    "CallLastSale",
    "CallNet",
    "CallBid",
    "CallAsk",
    "CallVol",
    "CallIV",
    "CallDelta",
    "CallGamma",
    "CallOpenInt",
    "StrikePrice",
    "PutLastSale",
    "PutNet",
    "PutBid",
    "PutAsk",
    "PutVol",
    "PutIV",
    "PutDelta",
    "PutGamma",
    "PutOpenInt",
    "CallVega",
    "PutVega",
    "CallTheta",
    "PutTheta",
    "_spot",
];

/// Assemble le lot Arrow d'une chaîne.
fn en_lot(chaine: &Chaine) -> Result<RecordBatch, ErreurReleve> {
    let n = chaine.lignes().len();
    let mut champs: Vec<Field> = vec![Field::new(
        "ExpirationDate",
        DataType::Timestamp(TimeUnit::Microsecond, None),
        false,
    )];
    let mut colonnes: Vec<arrow_array::ArrayRef> = vec![Arc::new(
        TimestampMicrosecondArray::from(
            chaine
                .lignes()
                .iter()
                .map(|l| l.echeance.0.and_utc().timestamp_micros())
                .collect::<Vec<_>>(),
        ),
    )];

    for nom in COLONNES_TEXTE {
        champs.push(Field::new(nom, DataType::Utf8, true));
        colonnes.push(Arc::new(StringArray::from(vec![""; n])));
    }

    for nom in COLONNES_NUM {
        let valeurs: Vec<f64> = chaine
            .lignes()
            .iter()
            .map(|l| match nom {
                "StrikePrice" => l.strike,
                "CallIV" => l.call.iv,
                "CallGamma" => l.call.gamma,
                "CallDelta" => l.call.delta,
                "CallOpenInt" => l.call.open_interest,
                "CallVega" => l.call.vega,
                "CallTheta" => l.call.theta,
                "PutIV" => l.put.iv,
                "PutGamma" => l.put.gamma,
                "PutDelta" => l.put.delta,
                "PutOpenInt" => l.put.open_interest,
                "PutVega" => l.put.vega,
                "PutTheta" => l.put.theta,
                // Constante sur toutes les lignes : c'est ainsi que le format
                // transporte le spot, et lire_releve le relit de la même façon.
                "_spot" => chaine.spot,
                _ => 0.0,
            })
            .collect();
        champs.push(Field::new(nom, DataType::Float64, true));
        colonnes.push(Arc::new(Float64Array::from(valeurs)));
    }

    champs.push(Field::new(
        "_quote_date",
        DataType::Timestamp(TimeUnit::Microsecond, None),
        false,
    ));
    colonnes.push(Arc::new(TimestampMicrosecondArray::from(vec![
        chaine.releve.0.and_utc().timestamp_micros();
        n
    ])));

    RecordBatch::try_new(Arc::new(Schema::new(champs)), colonnes)
        .map_err(ErreurReleve::Decodage)
}

/// Écrit une chaîne au chemin donné, en créant le dossier au besoin.
pub fn ecrire(chaine: &Chaine, cible: &Path) -> Result<(), ErreurReleve> {
    if let Some(dossier) = cible.parent() {
        create_dir_all(dossier).map_err(ErreurReleve::Fichier)?;
    }
    let lot = en_lot(chaine)?;
    let fichier = File::create(cible).map_err(ErreurReleve::Fichier)?;
    let mut plume =
        ArrowWriter::try_new(fichier, lot.schema(), None).map_err(ErreurReleve::Format)?;
    plume.write(&lot).map_err(ErreurReleve::Format)?;
    plume.close().map_err(ErreurReleve::Format)?;
    Ok(())
}

/// Réécrit le relevé courant, à chemin fixe.
pub fn ecrire_courant(
    chaine: &Chaine,
    dossier: &Path,
    produit: &str,
) -> Result<PathBuf, ErreurReleve> {
    let cible = chemin_courant(dossier, produit);
    ecrire(chaine, &cible)?;
    Ok(cible)
}

/// Écrit une archive horodatée à la minute.
///
/// Même horodatage que côté Python — `AAAA-MM-JJ_HHMM` — pour que les deux
/// familles de fichiers se trient ensemble dans un dossier déjà peuplé.
pub fn archiver(chaine: &Chaine, dossier: &Path, produit: &str) -> Result<PathBuf, ErreurReleve> {
    let cible = dossier
        .join(produit.to_ascii_uppercase())
        .join(format!(
            "{}.parquet",
            chaine.releve.0.format("%Y-%m-%d_%H%M")
        ));
    ecrire(chaine, &cible)?;
    Ok(cible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lire_releve;
    use chrono::NaiveDateTime;
    use gex_core::chaine::{Cote, Ligne};
    use gex_core::temps::{EcheanceNy, InstantReleve};

    fn instant(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn chaine_essai() -> Chaine {
        let lignes: Vec<Ligne> = [29_000.0_f64, 29_100.0, 29_200.0]
            .iter()
            .map(|k| Ligne {
                echeance: EcheanceNy(instant("2026-08-27 16:00:00")),
                strike: *k,
                call: Cote {
                    iv: 0.22,
                    gamma: 0.00086,
                    delta: 0.344,
                    open_interest: 300.0,
                    vega: 7.125,
                    theta: -58.47,
                },
                put: Cote {
                    iv: 0.24,
                    gamma: 0.00086,
                    delta: -0.656,
                    open_interest: 900.0,
                    vega: 7.0,
                    theta: -55.0,
                },
            })
            .collect();
        Chaine::nouvelle(
            lignes,
            29_227.89,
            InstantReleve(instant("2026-08-26 05:52:14")),
        )
        .unwrap()
    }

    fn dossier(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(nom);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// Le seul test qui compte vraiment : ce qu'on écrit doit se relire à
    /// l'identique. Le format est le joint entre le collecteur et tout ce qui
    /// relira ces fichiers, y compris dans six mois.
    #[test]
    fn ce_qui_est_ecrit_se_relit_a_l_identique() {
        let d = dossier("gex-test-ecriture");
        let avant = chaine_essai();
        let cible = ecrire_courant(&avant, &d, "NQ").unwrap();

        let apres = lire_releve(&cible).unwrap();
        assert_eq!(apres.lignes().len(), avant.lignes().len());
        assert!((apres.spot - avant.spot).abs() < 1e-9);
        assert_eq!(apres.releve, avant.releve);
        for (a, b) in apres.lignes().iter().zip(avant.lignes()) {
            assert_eq!(a.echeance, b.echeance);
            assert!((a.strike - b.strike).abs() < 1e-9);
            assert!((a.call.iv - b.call.iv).abs() < 1e-12);
            assert!((a.call.gamma - b.call.gamma).abs() < 1e-12);
            assert!((a.call.open_interest - b.call.open_interest).abs() < 1e-9);
            assert!((a.put.theta - b.put.theta).abs() < 1e-12);
            assert!((a.put.delta - b.put.delta).abs() < 1e-12);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// L'heure de règlement doit survivre : c'est le défaut qui avait coûté
    /// dix-neuf pour cent du GEX quand l'échéance arrivait sans elle.
    #[test]
    fn l_heure_de_reglement_survit_a_l_ecriture() {
        let d = dossier("gex-test-heure");
        let cible = ecrire_courant(&chaine_essai(), &d, "NQ").unwrap();
        let relu = lire_releve(&cible).unwrap();
        assert_eq!(
            relu.lignes()[0].echeance.0.format("%H:%M").to_string(),
            "16:00"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Le courant est réécrit EN PLACE : sinon près de mille cinq cents fichiers
    /// par jour, et un lecteur incapable de savoir lequel est le dernier.
    #[test]
    fn le_courant_est_reecrit_en_place() {
        let d = dossier("gex-test-courant");
        let un = ecrire_courant(&chaine_essai(), &d, "NQ").unwrap();
        let deux = ecrire_courant(&chaine_essai(), &d, "NQ").unwrap();
        assert_eq!(un, deux);
        let fichiers = std::fs::read_dir(d.join("NQ")).unwrap().count();
        assert_eq!(fichiers, 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// L'archive est horodatée à la minute, au format que Python écrivait : les
    /// deux familles de fichiers doivent se trier ensemble.
    #[test]
    fn l_archive_est_horodatee_a_la_minute() {
        let d = dossier("gex-test-archive");
        let cible = archiver(&chaine_essai(), &d, "NQ").unwrap();
        assert!(
            cible.ends_with("2026-08-26_0552.parquet"),
            "horodatage inattendu : {cible:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
