//! Les deux séries qu'un écran de séance demande : les barres et les niveaux.
//!
//! Le calcul rend un zero gamma, on l'affiche, on le jette. Or **c'est la dérive
//! qui porte l'information** : un zero gamma à 29 400 ne dit rien seul, le voir
//! monter de 29 200 pendant que le prix s'en approche, si.
//!
//! Deux fichiers et non un, parce que ce sont deux durées de vie. Les barres
//! viennent du future et existent même quand aucune option n'a été collectée ; les
//! niveaux viennent de la chaîne et n'existent que quand un socle est là. Les
//! fondre obligerait à porter des trous dans les deux sens.
//!
//! Les deux partagent le même axe de temps — un point par minute, en UTC — ce qui
//! évite d'aligner deux échelles à l'affichage.

use std::fs::{create_dir_all, rename};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Array, Float64Array, RecordBatch, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use chrono::{DateTime, Duration, NaiveDateTime, Timelike};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::ErreurReleve;

/// Jours de série conservés par défaut.
///
/// Une série qui grossit sans fin est un piège différé : les archives horodatées
/// ont été désactivées précisément parce qu'elles s'accumulaient sans que rien ne
/// les efface. Trente jours de barres à la minute font environ 43 000 lignes.
pub const RETENTION_JOURS: i64 = 30;

/// Une barre de prix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Barre {
    /// Début de la barre, en UTC.
    pub instant: NaiveDateTime,
    /// Prix d'ouverture.
    pub open: f64,
    /// Plus haut.
    pub high: f64,
    /// Plus bas.
    pub low: f64,
    /// Clôture.
    pub close: f64,
    /// Volume échangé.
    pub volume: f64,
}

/// Un point de la série des niveaux.
///
/// Le strict nécessaire pour tracer. Pas de profil ni de GEX par strike : ce sont
/// deux ordres de grandeur de plus par minute, pour quelque chose que l'écran lira
/// depuis le relevé courant — le profil ne s'affiche qu'à l'instant présent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointNiveaux {
    /// L'instant du calcul, en UTC.
    pub instant: NaiveDateTime,
    /// Le prix au moment du calcul, pour recouper avec les barres.
    pub spot: f64,
    /// Le niveau où le gamma change de signe.
    pub zero_gamma: Option<f64>,
    /// GEX total : l'histogramme du bas, et le signe du régime.
    pub gex: f64,
    /// Charm total.
    pub charm: f64,
    /// Vanna totale.
    pub vanna: f64,
    /// Delta, vega et thêta dollar du book, les trois greeks de POSITION.
    ///
    /// `Option` et non `f64` comme leurs voisins, pour une raison qui n'est pas
    /// cosmétique : ils ne sont pas recalculés faute d'être publiés, et un fichier
    /// écrit avant leur ajout n'en a aucune trace. Les relire à zéro dessinerait
    /// une ligne plate qui se lit « le book était neutre » sur toute la séance
    /// précédente. Vide veut dire vide.
    pub delta: Option<f64>,
    /// Vega dollar du book, par point de volatilité.
    pub vega: Option<f64>,
    /// Thêta dollar du book, par jour.
    pub theta: Option<f64>,
    /// L'horizon d'échéance sous lequel ce point a été calculé, en jours.
    ///
    /// La série devient ainsi **auto-descriptive**, comme l'historique CSV qui
    /// porte déjà sa colonne `dte_max`. Sans elle, un écran qui laisse choisir un
    /// horizon ne peut pas dire sous quel horizon la trace a été tracée — et deux
    /// relevés d'horizons différents ne sont pas comparables : sur un indice, la
    /// chaîne entière et le 0–7 DTE donnent des GEX de signes opposés.
    ///
    /// `None` sur les fichiers écrits avant son ajout. Le collecteur écrit
    /// toujours un nombre : son `--dte-max` n'a pas de variante « toutes ».
    pub dte_max: Option<f64>,
    /// Mur call en gamma.
    pub call_wall: Option<f64>,
    /// Mur put en gamma.
    pub put_wall: Option<f64>,
    /// Mur call en open interest brut.
    pub call_wall_oi: Option<f64>,
    /// Mur put en open interest brut.
    pub put_wall_oi: Option<f64>,
    /// La volatilité implicite à la monnaie, sur l'échéance la plus proche.
    ///
    /// Elle n'est pas un prix et ne se trace pas sur l'axe du sous-jacent. Elle
    /// est là parce que le vanna ne veut rien dire sans elle : une exposition
    /// « par point de volatilité » ne compte que si la volatilité bouge.
    pub iv_atm: Option<f64>,
    /// La pente du smile sur cette même échéance.
    pub skew: Option<f64>,
    /// Le strike où les options de l'échéance la plus proche valent le moins au
    /// règlement.
    ///
    /// Il dérive lentement : c'est l'open interest qui bouge, pas le prix. Une
    /// série le montre là où un chiffre du jour ne dirait rien — un max pain qui
    /// se déplace vers le spot n'a pas le même sens qu'un max pain immobile que
    /// le prix rejoint.
    pub max_pain: Option<f64>,
}

/// La minute a-t-elle changé depuis le dernier point écrit ?
///
/// Comparer des durées écoulées dériverait : le collecteur tourne toutes les
/// quinze secondes, et quatre tours font parfois cinquante-neuf secondes — un
/// point sur quatre sauterait sa minute. On compare donc la minute elle-même.
pub fn faut_il_ecrire_un_point(dernier: Option<NaiveDateTime>, maintenant: NaiveDateTime) -> bool {
    match dernier {
        None => true,
        Some(avant) => {
            avant.date() != maintenant.date()
                || avant.hour() != maintenant.hour()
                || avant.minute() != maintenant.minute()
        }
    }
}

/// L'instant ramené au début de sa minute.
///
/// Les deux séries doivent partager le même axe de temps, et les barres d'IB
/// tombent sur la minute pile. Un point de niveaux à 07:45:46,160 ne se
/// superposerait à aucune d'elles : l'écran aurait deux échelles à aligner, ou
/// tracerait des points entre les chandeliers.
pub fn a_la_minute(instant: NaiveDateTime) -> NaiveDateTime {
    instant
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(instant)
}

/// L'instant à partir duquel on garde, en jours glissants.
pub fn borne_de_retention(maintenant: NaiveDateTime, jours: i64) -> NaiveDateTime {
    maintenant - Duration::days(jours.max(0))
}

/// Recolle des barres nouvelles sur des anciennes.
///
/// IB renvoie la **dernière barre plusieurs fois** pendant qu'elle se forme : la
/// même minute arrive incomplète, puis complète. Les ajouter produirait des
/// doublons dont le dernier seul est juste ; on écrase donc par horodatage.
///
/// Le résultat est trié, parce que le flux d'IB ne garantit pas l'ordre après une
/// reconnexion — l'historique renvoyé recouvre ce qu'on avait déjà.
pub fn recoller(anciennes: &[Barre], nouvelles: &[Barre]) -> Vec<Barre> {
    let mut toutes: Vec<Barre> = anciennes.to_vec();
    for neuve in nouvelles {
        match toutes.iter_mut().find(|a| a.instant == neuve.instant) {
            Some(ancienne) => *ancienne = *neuve,
            None => toutes.push(*neuve),
        }
    }
    toutes.sort_by_key(|b| b.instant);
    toutes
}

/// Ne garde que ce qui tombe après la borne.
pub fn elaguer_barres(barres: &[Barre], borne: NaiveDateTime) -> Vec<Barre> {
    barres.iter().copied().filter(|b| b.instant >= borne).collect()
}

/// Ne garde que ce qui tombe après la borne.
pub fn elaguer_niveaux(points: &[PointNiveaux], borne: NaiveDateTime) -> Vec<PointNiveaux> {
    points.iter().copied().filter(|p| p.instant >= borne).collect()
}

/// Le chemin de la série des barres.
pub fn chemin_barres(dossier: impl AsRef<Path>, produit: &str) -> PathBuf {
    dossier
        .as_ref()
        .join(produit.to_ascii_uppercase())
        .join("barres.parquet")
}

/// Le chemin de la série des niveaux.
pub fn chemin_niveaux(dossier: impl AsRef<Path>, produit: &str) -> PathBuf {
    dossier
        .as_ref()
        .join(produit.to_ascii_uppercase())
        .join("niveaux.parquet")
}

/// Écrit un lot Arrow, par un fichier temporaire puis un renommage.
///
/// L'atomicité n'est pas un luxe : un lecteur qui ouvre pendant l'écriture verrait
/// un parquet incomplet, et un parquet incomplet se lit comme une séance qui
/// s'arrête — pas comme une erreur.
fn ecrire_atomique(lot: &RecordBatch, cible: &Path) -> Result<(), ErreurReleve> {
    if let Some(dossier) = cible.parent() {
        create_dir_all(dossier).map_err(ErreurReleve::Fichier)?;
    }
    let provisoire = cible.with_extension("parquet.tmp");
    {
        let fichier = std::fs::File::create(&provisoire).map_err(ErreurReleve::Fichier)?;
        let mut plume =
            ArrowWriter::try_new(fichier, lot.schema(), None).map_err(ErreurReleve::Format)?;
        plume.write(lot).map_err(ErreurReleve::Format)?;
        plume.close().map_err(ErreurReleve::Format)?;
    }
    rename(&provisoire, cible).map_err(ErreurReleve::Fichier)
}

/// Les colonnes d'un instant et de flottants, assemblées en lot Arrow.
fn lot(
    instants: Vec<i64>,
    colonnes: Vec<(&str, Vec<Option<f64>>)>,
) -> Result<RecordBatch, ErreurReleve> {
    let mut champs = vec![Field::new(
        "instant",
        DataType::Timestamp(TimeUnit::Microsecond, None),
        false,
    )];
    let mut valeurs: Vec<arrow_array::ArrayRef> =
        vec![Arc::new(TimestampMicrosecondArray::from(instants))];
    for (nom, v) in colonnes {
        champs.push(Field::new(nom, DataType::Float64, true));
        valeurs.push(Arc::new(Float64Array::from(v)));
    }
    RecordBatch::try_new(Arc::new(Schema::new(champs)), valeurs).map_err(ErreurReleve::Decodage)
}

/// Écrit la série des barres.
pub fn ecrire_barres(barres: &[Barre], cible: &Path) -> Result<(), ErreurReleve> {
    let instants = barres.iter().map(|b| b.instant.and_utc().timestamp_micros()).collect();
    let colonne = |extrait: fn(&Barre) -> f64| -> Vec<Option<f64>> {
        barres.iter().map(|b| Some(extrait(b))).collect()
    };
    let lot = lot(
        instants,
        vec![
            ("open", colonne(|b| b.open)),
            ("high", colonne(|b| b.high)),
            ("low", colonne(|b| b.low)),
            ("close", colonne(|b| b.close)),
            ("volume", colonne(|b| b.volume)),
        ],
    )?;
    ecrire_atomique(&lot, cible)
}

/// Écrit la série des niveaux.
pub fn ecrire_niveaux(points: &[PointNiveaux], cible: &Path) -> Result<(), ErreurReleve> {
    let instants = points.iter().map(|p| p.instant.and_utc().timestamp_micros()).collect();
    let sur = |extrait: fn(&PointNiveaux) -> f64| -> Vec<Option<f64>> {
        points.iter().map(|p| Some(extrait(p))).collect()
    };
    // Un niveau absent laisse la case vide, jamais un zéro : zéro est un prix, et
    // la relecture le prendrait pour un zero gamma au plancher.
    let peut_etre = |extrait: fn(&PointNiveaux) -> Option<f64>| -> Vec<Option<f64>> {
        points.iter().map(extrait).collect()
    };
    let lot = lot(
        instants,
        vec![
            ("spot", sur(|p| p.spot)),
            ("zero_gamma", peut_etre(|p| p.zero_gamma)),
            ("gex", sur(|p| p.gex)),
            ("charm", sur(|p| p.charm)),
            ("vanna", sur(|p| p.vanna)),
            ("call_wall", peut_etre(|p| p.call_wall)),
            ("put_wall", peut_etre(|p| p.put_wall)),
            ("call_wall_oi", peut_etre(|p| p.call_wall_oi)),
            ("put_wall_oi", peut_etre(|p| p.put_wall_oi)),
            ("iv_atm", peut_etre(|p| p.iv_atm)),
            ("skew", peut_etre(|p| p.skew)),
            ("max_pain", peut_etre(|p| p.max_pain)),
            ("delta", peut_etre(|p| p.delta)),
            ("vega", peut_etre(|p| p.vega)),
            ("theta", peut_etre(|p| p.theta)),
            ("dte_max", peut_etre(|p| p.dte_max)),
        ],
    )?;
    ecrire_atomique(&lot, cible)
}

/// Ce qu'une lecture de série rend : l'axe de temps, puis une colonne de valeurs
/// par nom demandé, dans le même ordre.
type Colonnes = (Vec<NaiveDateTime>, Vec<Vec<Option<f64>>>);

/// Lit un parquet de série : les instants, et les colonnes nommées.
///
/// Un fichier absent rend une série vide plutôt qu'une erreur : au tout premier
/// démarrage il n'y a rien à lire, et ce n'est pas une panne.
fn lire_colonnes(
    source: &Path,
    noms: &[&str],
) -> Result<Colonnes, ErreurReleve> {
    if !source.exists() {
        return Ok((Vec::new(), noms.iter().map(|_| Vec::new()).collect()));
    }
    let fichier = std::fs::File::open(source).map_err(ErreurReleve::Fichier)?;
    let lecteur = ParquetRecordBatchReaderBuilder::try_new(fichier)
        .map_err(ErreurReleve::Format)?
        .build()
        .map_err(ErreurReleve::Format)?;

    let mut instants = Vec::new();
    let mut colonnes: Vec<Vec<Option<f64>>> = noms.iter().map(|_| Vec::new()).collect();
    for lot in lecteur {
        let lot = lot.map_err(ErreurReleve::Decodage)?;
        let horodatages = lot
            .column_by_name("instant")
            .and_then(|c| c.as_any().downcast_ref::<TimestampMicrosecondArray>())
            .ok_or(ErreurReleve::ColonneAbsente("instant"))?;
        for i in 0..horodatages.len() {
            instants.push(
                DateTime::from_timestamp_micros(horodatages.value(i))
                    .ok_or(ErreurReleve::TypeInattendu("instant"))?
                    .naive_utc(),
            );
        }
        for (rang, nom) in noms.iter().enumerate() {
            let valeurs = lot
                .column_by_name(nom)
                .and_then(|c| c.as_any().downcast_ref::<Float64Array>());
            match valeurs {
                None => colonnes[rang].extend(std::iter::repeat_n(None, lot.num_rows())),
                Some(v) => {
                    for i in 0..v.len() {
                        colonnes[rang].push((!v.is_null(i)).then(|| v.value(i)));
                    }
                }
            }
        }
    }
    Ok((instants, colonnes))
}

/// Relit la série des barres.
pub fn lire_barres(source: &Path) -> Result<Vec<Barre>, ErreurReleve> {
    let (instants, c) = lire_colonnes(source, &["open", "high", "low", "close", "volume"])?;
    Ok(instants
        .into_iter()
        .enumerate()
        .map(|(i, instant)| Barre {
            instant,
            open: c[0][i].unwrap_or(0.0),
            high: c[1][i].unwrap_or(0.0),
            low: c[2][i].unwrap_or(0.0),
            close: c[3][i].unwrap_or(0.0),
            volume: c[4][i].unwrap_or(0.0),
        })
        .collect())
}

/// Relit la série des niveaux.
pub fn lire_niveaux(source: &Path) -> Result<Vec<PointNiveaux>, ErreurReleve> {
    let (instants, c) = lire_colonnes(
        source,
        &[
            "spot",
            "zero_gamma",
            "gex",
            "charm",
            "vanna",
            "call_wall",
            "put_wall",
            "call_wall_oi",
            "put_wall_oi",
            "iv_atm",
            "skew",
            "max_pain",
            "delta",
            "vega",
            "theta",
            "dte_max",
        ],
    )?;
    Ok(instants
        .into_iter()
        .enumerate()
        .map(|(i, instant)| PointNiveaux {
            instant,
            spot: c[0][i].unwrap_or(0.0),
            zero_gamma: c[1][i],
            gex: c[2][i].unwrap_or(0.0),
            charm: c[3][i].unwrap_or(0.0),
            vanna: c[4][i].unwrap_or(0.0),
            call_wall: c[5][i],
            put_wall: c[6][i],
            call_wall_oi: c[7][i],
            put_wall_oi: c[8][i],
            // Absentes des fichiers ecrits avant leur ajout : la lecture rend
            // alors None, et la serie reste lisible.
            iv_atm: c[9][i],
            skew: c[10][i],
            max_pain: c[11][i],
            delta: c[12][i],
            vega: c[13][i],
            theta: c[14][i],
            dte_max: c[15][i],
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn barre(quand: &str, close: f64) -> Barre {
        Barre {
            instant: instant(quand),
            open: close - 5.0,
            high: close + 8.0,
            low: close - 9.0,
            close,
            volume: 1_200.0,
        }
    }

    fn point(quand: &str, zero: Option<f64>) -> PointNiveaux {
        PointNiveaux {
            instant: instant(quand),
            spot: 29_218.5,
            zero_gamma: zero,
            gex: -478_840_000.0,
            charm: -154_030_000.0,
            vanna: 9_100_000.0,
            call_wall: None,
            put_wall: Some(29_050.0),
            call_wall_oi: None,
            iv_atm: None,
            skew: None,
            max_pain: Some(29_100.0),
            delta: Some(15_470_400_000.0),
            vega: Some(-962_400.0),
            theta: Some(-1_204_000.0),
            dte_max: Some(30.0),
            put_wall_oi: Some(29_000.0),
        }
    }

    fn dossier(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(nom);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    // ---=== la cadence ===---

    /// Comparer des durées écoulées dériverait : à quinze secondes d'intervalle,
    /// quatre tours font parfois cinquante-neuf secondes.
    #[test]
    fn un_point_par_minute_pas_par_duree_ecoulee() {
        let avant = instant("2026-08-26 09:15:02");
        assert!(!faut_il_ecrire_un_point(Some(avant), instant("2026-08-26 09:15:59")));
        assert!(faut_il_ecrire_un_point(Some(avant), instant("2026-08-26 09:16:01")));
        // Cinquante-neuf secondes plus tard mais dans la minute suivante : on écrit.
        assert!(faut_il_ecrire_un_point(
            Some(instant("2026-08-26 09:15:58")),
            instant("2026-08-26 09:16:00")
        ));
    }

    /// Les barres d'IB tombent sur la minute pile : un point à 07:45:46,160 ne se
    /// superposerait à aucune d'elles.
    #[test]
    fn un_instant_est_ramene_au_debut_de_sa_minute() {
        assert_eq!(
            a_la_minute(instant("2026-08-26 07:45:46")),
            instant("2026-08-26 07:45:00")
        );
        // Déjà sur la minute : rien ne change.
        assert_eq!(
            a_la_minute(instant("2026-08-26 07:45:00")),
            instant("2026-08-26 07:45:00")
        );
    }

    #[test]
    fn le_premier_point_s_ecrit_toujours() {
        assert!(faut_il_ecrire_un_point(None, instant("2026-08-26 09:15:02")));
    }

    /// Le passage d'heure et de jour ne doit pas passer pour la même minute.
    #[test]
    fn le_changement_d_heure_ou_de_jour_compte() {
        assert!(faut_il_ecrire_un_point(
            Some(instant("2026-08-26 09:15:30")),
            instant("2026-08-26 10:15:30")
        ));
        assert!(faut_il_ecrire_un_point(
            Some(instant("2026-08-26 09:15:30")),
            instant("2026-08-27 09:15:30")
        ));
    }

    // ---=== le recollage ===---

    /// IB renvoie la dernière barre plusieurs fois pendant qu'elle se forme : les
    /// ajouter produirait des doublons dont le dernier seul est juste.
    #[test]
    fn une_barre_qui_se_forme_ecrase_sa_version_precedente() {
        let anciennes = vec![barre("2026-08-26 09:15:00", 29_200.0)];
        let nouvelles = vec![barre("2026-08-26 09:15:00", 29_250.0)];
        let recollees = recoller(&anciennes, &nouvelles);
        assert_eq!(recollees.len(), 1);
        assert_eq!(recollees[0].close, 29_250.0);
    }

    /// Après une reconnexion, IB renvoie l'historique avant de reprendre le
    /// direct : il recouvre ce qu'on avait, et l'ordre n'est pas garanti.
    #[test]
    fn le_recollage_trie_et_comble() {
        let anciennes = vec![
            barre("2026-08-26 09:15:00", 29_200.0),
            barre("2026-08-26 09:16:00", 29_210.0),
        ];
        let nouvelles = vec![
            barre("2026-08-26 09:18:00", 29_230.0),
            barre("2026-08-26 09:17:00", 29_220.0),
            barre("2026-08-26 09:16:00", 29_215.0),
        ];
        let recollees = recoller(&anciennes, &nouvelles);
        assert_eq!(recollees.len(), 4);
        assert!(recollees.windows(2).all(|p| p[0].instant < p[1].instant));
        assert_eq!(recollees[1].close, 29_215.0, "la version fraîche l'emporte");
    }

    // ---=== la rétention ===---

    /// Une série qui grossit sans fin est un piège différé.
    #[test]
    fn l_elagage_borne_la_serie() {
        let barres = vec![
            barre("2026-07-01 09:15:00", 28_000.0),
            barre("2026-08-20 09:15:00", 29_000.0),
            barre("2026-08-26 09:15:00", 29_200.0),
        ];
        let borne = borne_de_retention(instant("2026-08-26 12:00:00"), RETENTION_JOURS);
        let gardees = elaguer_barres(&barres, borne);
        assert_eq!(gardees.len(), 2, "le 1er juillet est hors des trente jours");
        assert_eq!(gardees[0].close, 29_000.0);
    }

    #[test]
    fn une_retention_nulle_ne_garde_que_le_present() {
        let borne = borne_de_retention(instant("2026-08-26 12:00:00"), 0);
        assert_eq!(borne, instant("2026-08-26 12:00:00"));
    }

    #[test]
    fn l_elagage_des_niveaux_suit_la_meme_borne() {
        let points = vec![
            point("2026-07-01 09:15:00", Some(28_000.0)),
            point("2026-08-26 09:15:00", Some(29_400.0)),
        ];
        let borne = borne_de_retention(instant("2026-08-26 12:00:00"), RETENTION_JOURS);
        assert_eq!(elaguer_niveaux(&points, borne).len(), 1);
    }

    // ---=== l'aller-retour ===---

    #[test]
    fn les_barres_se_relisent_a_l_identique() {
        let d = dossier("gex-test-barres");
        let cible = chemin_barres(&d, "NQ");
        let barres = vec![
            barre("2026-08-26 09:15:00", 29_200.0),
            barre("2026-08-26 09:16:00", 29_210.5),
        ];
        ecrire_barres(&barres, &cible).unwrap();
        let relues = lire_barres(&cible).unwrap();
        assert_eq!(relues, barres);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Un niveau absent laisse la case vide, jamais un zéro : zéro est un prix, et
    /// la relecture le prendrait pour un zero gamma au plancher.
    #[test]
    fn un_niveau_absent_se_relit_absent() {
        let d = dossier("gex-test-niveaux");
        let cible = chemin_niveaux(&d, "NQ");
        let points = vec![
            point("2026-08-26 09:15:00", None),
            point("2026-08-26 09:16:00", Some(29_400.0)),
        ];
        ecrire_niveaux(&points, &cible).unwrap();
        let relus = lire_niveaux(&cible).unwrap();
        assert_eq!(relus, points);
        assert_eq!(relus[0].zero_gamma, None);
        assert_eq!(relus[0].call_wall, None);
        assert_eq!(relus[1].zero_gamma, Some(29_400.0));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Une série écrite avant l'ajout d'une colonne reste lisible.
    ///
    /// Les points déjà accumulés n'ont pas de max pain et n'en auront jamais : le
    /// relevé qui les a produits n'existe plus. La relecture rend `None` pour
    /// eux, et la séance en cours continue de s'écrire par-dessus — sans quoi un
    /// ajout de mesure effacerait l'historique de celles d'avant.
    #[test]
    fn un_ancien_fichier_sans_la_colonne_reste_lisible() {
        let d = dossier("gex-test-ancienne-colonne");
        let cible = chemin_niveaux(&d, "NQ");
        let p = point("2026-08-26 09:15:00", Some(29_400.0));

        // Le schéma d'avant : tout sauf `max_pain`.
        let ancien = lot(
            vec![p.instant.and_utc().timestamp_micros()],
            vec![
                ("spot", vec![Some(p.spot)]),
                ("zero_gamma", vec![p.zero_gamma]),
                ("gex", vec![Some(p.gex)]),
                ("charm", vec![Some(p.charm)]),
                ("vanna", vec![Some(p.vanna)]),
                ("call_wall", vec![p.call_wall]),
                ("put_wall", vec![p.put_wall]),
                ("call_wall_oi", vec![p.call_wall_oi]),
                ("put_wall_oi", vec![p.put_wall_oi]),
                ("iv_atm", vec![p.iv_atm]),
                ("skew", vec![p.skew]),
            ],
        )
        .unwrap();
        ecrire_atomique(&ancien, &cible).unwrap();

        let relus = lire_niveaux(&cible).unwrap();
        assert_eq!(relus.len(), 1);
        assert_eq!(relus[0].max_pain, None, "colonne absente, pas un zéro");
        assert_eq!(relus[0].delta, None);
        assert_eq!(relus[0].theta, None);
        // L'horizon inconnu doit rester inconnu : l'écran dit « non enregistré »
        // plutôt que de supposer celui qu'il affiche lui-même.
        assert_eq!(relus[0].dte_max, None, "horizon inconnu, pas supposé");
        assert_eq!(relus[0].zero_gamma, Some(29_400.0), "le reste est intact");
        assert_eq!(relus[0].put_wall_oi, p.put_wall_oi);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Au tout premier démarrage il n'y a rien à lire, et ce n'est pas une panne.
    #[test]
    fn une_serie_absente_se_lit_vide() {
        let d = dossier("gex-test-absent");
        assert!(lire_barres(&chemin_barres(&d, "NQ")).unwrap().is_empty());
        assert!(lire_niveaux(&chemin_niveaux(&d, "NQ")).unwrap().is_empty());
    }

    /// Un lecteur qui ouvre pendant l'écriture verrait un parquet incomplet, et un
    /// parquet incomplet se lit comme une séance qui s'arrête.
    #[test]
    fn l_ecriture_ne_laisse_pas_de_fichier_provisoire() {
        let d = dossier("gex-test-atomique");
        let cible = chemin_barres(&d, "NQ");
        ecrire_barres(&[barre("2026-08-26 09:15:00", 29_200.0)], &cible).unwrap();
        let restes: Vec<_> = std::fs::read_dir(cible.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(restes.is_empty(), "un fichier provisoire est resté");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn une_serie_vide_s_ecrit_et_se_relit() {
        let d = dossier("gex-test-vide");
        let cible = chemin_barres(&d, "NQ");
        ecrire_barres(&[], &cible).unwrap();
        assert!(lire_barres(&cible).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
