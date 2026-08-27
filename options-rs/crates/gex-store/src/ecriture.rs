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
                // `CallVol` et `PutVol` existent dans le format depuis le CBOE et
                // s'écrivaient à zéro faute d'être collectées. Elles portent le
                // volume du jour : aucune colonne à ajouter, juste à remplir.
                "CallVol" => l.call.volume,
                "PutVol" => l.put.volume,
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

/// Le format d'horodatage porté par le nom d'une archive.
const HORODATAGE: &str = "%Y-%m-%d_%H%M";

/// Efface les archives antérieures à la borne, et rend leur nombre.
///
/// Sans elle, `--archiver` était un piège différé : le collecteur écrivait un
/// relevé de 245 Ko à chaque cadence et **rien ne l'effaçait jamais**. À la
/// minute, cela fait 353 Mo par jour — la même fenêtre glissante de trente jours
/// que les séries coûte 10,6 Go, un chiffre stable ; sans élagage, c'est 10,6 Go
/// de plus chaque mois, indéfiniment.
///
/// C'est le **nom** qui décide de l'âge, pas la date du fichier : une copie, une
/// restauration ou une horloge remise à l'heure changeraient la seconde, jamais
/// le premier.
///
/// Et seuls les noms de cette forme sont candidats. `courant.parquet`,
/// `barres.parquet` et `niveaux.parquet` vivent dans le même dossier ; un
/// balayage qui les prendrait pour des archives effacerait la séance en cours.
pub fn elaguer_archives(
    dossier: &Path,
    produit: &str,
    borne: chrono::NaiveDateTime,
) -> Result<usize, ErreurReleve> {
    let dossier = dossier.join(produit.to_ascii_uppercase());
    if !dossier.exists() {
        return Ok(0);
    }
    let mut effacees = 0;
    for entree in std::fs::read_dir(&dossier).map_err(ErreurReleve::Fichier)? {
        let chemin = entree.map_err(ErreurReleve::Fichier)?.path();
        if chemin.extension().is_none_or(|e| e != "parquet") {
            continue;
        }
        let Some(nom) = chemin.file_stem().and_then(|n| n.to_str()) else {
            continue;
        };
        // Un nom qui n'est pas un horodatage n'est pas une archive : on n'y
        // touche pas, quel qu'il soit.
        let Ok(quand) = chrono::NaiveDateTime::parse_from_str(nom, HORODATAGE) else {
            continue;
        };
        if quand < borne {
            std::fs::remove_file(&chemin).map_err(ErreurReleve::Fichier)?;
            effacees += 1;
        }
    }
    Ok(effacees)
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
                    volume: 120.0,
                },
                put: Cote {
                    iv: 0.24,
                    gamma: 0.00086,
                    delta: -0.656,
                    open_interest: 900.0,
                    vega: 7.0,
                    theta: -55.0,
                    volume: 240.0,
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
            // Le volume passe par `CallVol` et `PutVol`, colonnes héritées du
            // CBOE longtemps écrites à zéro. Sans cette vérification, un
            // câblage manquant côté écriture OU côté lecture rendrait un volume
            // nul partout — et se lirait comme « la source ne le sert pas ».
            assert!((a.call.volume - b.call.volume).abs() < 1e-9, "volume call perdu");
            assert!((a.put.volume - b.put.volume).abs() < 1e-9, "volume put perdu");
            assert!(b.call.volume > 0.0, "la chaîne d'essai doit en porter");
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

    /// Une archive plus vieille que la borne s'en va, une récente reste.
    ///
    /// Sans cet élagage, `--archiver` accumulait 353 Mo par jour que rien
    /// n'effaçait jamais.
    #[test]
    fn les_archives_trop_vieilles_s_effacent() {
        let d = dossier("gex-test-elagage");
        let nq = d.join("NQ");
        std::fs::create_dir_all(&nq).unwrap();
        for nom in ["2026-07-01_0900", "2026-08-20_1430", "2026-08-26_0552"] {
            std::fs::write(nq.join(format!("{nom}.parquet")), b"x").unwrap();
        }
        let borne = instant("2026-08-25 00:00:00");
        assert_eq!(elaguer_archives(&d, "NQ", borne).unwrap(), 2);
        assert!(nq.join("2026-08-26_0552.parquet").exists());
        assert!(!nq.join("2026-07-01_0900.parquet").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Les trois fichiers de la séance vivent dans le même dossier. Un élagage
    /// qui les prendrait pour des archives effacerait le relevé courant, les
    /// barres et les niveaux — c'est-à-dire tout.
    #[test]
    fn l_elagage_ne_touche_pas_les_fichiers_de_seance() {
        let d = dossier("gex-test-elagage-seance");
        let nq = d.join("NQ");
        std::fs::create_dir_all(&nq).unwrap();
        for nom in ["courant", "barres", "niveaux", "notes"] {
            std::fs::write(nq.join(format!("{nom}.parquet")), b"x").unwrap();
        }
        std::fs::write(nq.join("2026-07-01_0900.parquet"), b"x").unwrap();

        // Une borne très postérieure : tout serait effacé si le nom ne décidait pas.
        let efface = elaguer_archives(&d, "NQ", instant("2030-01-01 00:00:00")).unwrap();
        assert_eq!(efface, 1, "seule l'archive horodatée devait partir");
        for nom in ["courant", "barres", "niveaux", "notes"] {
            assert!(nq.join(format!("{nom}.parquet")).exists(), "{nom} effacé");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Un dossier qui n'existe pas encore n'est pas une erreur : au tout premier
    /// démarrage il n'y a rien à élaguer.
    #[test]
    fn un_dossier_absent_n_elague_rien() {
        let d = dossier("gex-test-elagage-absent");
        assert_eq!(
            elaguer_archives(&d, "NQ", instant("2026-08-25 00:00:00")).unwrap(),
            0
        );
    }
}
