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
    /// Les murs vus par le VOLUME du jour.
    ///
    /// Troisième paire, et pas un doublon : l'open interest compte les positions
    /// accumulées depuis des semaines, le volume ce qui vient de se traiter. Un
    /// mur par volume apparaît et disparaît dans la journée là où un mur par open
    /// interest met des jours à bouger.
    /// Mur call par le volume du jour.
    pub call_wall_vol: Option<f64>,
    /// Mur put par le volume du jour.
    pub put_wall_vol: Option<f64>,
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
    /// Les deux strikes où le gamma **net** est le plus concentré.
    ///
    /// Ils n'étaient pas dans la série : l'écran les tirait du relevé courant, et
    /// ils n'avaient donc aucune trace. C'était une lacune circulaire — on ne
    /// pouvait pas savoir s'ils méritaient leurs deux lignes, faute de pouvoir
    /// mesurer à quelle fréquence ils se séparent des murs gamma. Les enregistrer
    /// tranche la question au lieu de la discuter.
    ///
    /// À ne pas confondre avec les murs, qui répondent à autre chose : un mur est
    /// calculé d'un seul côté du spot, ceux-ci prennent le GEX net sans regarder
    /// de quel côté il tombe.
    pub long_gamma: Option<f64>,
    /// Le strike au GEX net le plus négatif.
    pub short_gamma: Option<f64>,
}

/// L'écart toléré entre le prix servi par la source et le dernier prix traité.
///
/// Un demi pour cent, soit environ 145 points sur le NQ. Le seuil est choisi
/// large à dessein : les deux chiffres ne viennent pas du même mécanisme — le
/// premier des ticks d'option, le second des barres historiques — et un écart de
/// quelques points en séance rapide n'a rien d'anormal. Il ne s'agit pas de
/// mesurer une divergence fine mais d'attraper une source qui a cessé de servir.
///
/// La panne du 3 septembre 2026 valait **1,2 %** au moment où elle a été vue, et
/// elle durait depuis onze heures. Ce seuil l'aurait signalée dans la minute.
pub const ECART_TOLERE: f64 = 0.005;

/// Le prix servi est-il encore crédible face au dernier prix traité ?
///
/// # Pourquoi cette confrontation existe
///
/// Le spot du collecteur n'est pas la cotation du future : c'est la **médiane des
/// `undPrice`** qu'IB glisse dans chaque tick d'option. Quand la source cesse de
/// rafraîchir ces ticks, les valeurs restent *présentes* mais périmées — et un
/// code qui ne teste que leur présence continue de publier sans rien remarquer.
///
/// C'est exactement ce qui s'est produit le 3 septembre 2026 : onze heures de
/// niveaux calculés contre un prix de 29 152 pendant que le sous-jacent traitait
/// à 29 511. L'écran fonctionnait, servait fidèlement des chiffres faux, et rien
/// dans le dépôt ne pouvait le dire.
///
/// Les barres, elles, viennent de requêtes **historiques** — un tout autre
/// mécanisme, resté vivant pendant toute la panne. Les confronter est donc le
/// seul contrôle disponible qui ne dépende pas de ce qui est tombé.
///
/// Rend `None` quand tout va bien, ou quand il n'y a rien à comparer : sans
/// barre, l'absence de contrôle ne doit pas se lire comme un contrôle réussi.
pub fn spot_perime(spot: f64, barres: &[Barre]) -> Option<Perime> {
    let dernier = barres.last()?;
    if spot <= 0.0 || dernier.close <= 0.0 {
        return None;
    }
    let fraction = (spot - dernier.close).abs() / spot;
    (fraction > ECART_TOLERE).then_some(Perime {
        spot,
        dernier_traite: dernier.close,
        instant_traite: dernier.instant,
        fraction,
    })
}

/// Ce qu'on sait d'un prix qui a cessé de suivre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Perime {
    /// Le prix servi par la source, celui dont on doute.
    pub spot: f64,
    /// La clôture de la dernière barre, qui elle a continué d'arriver.
    pub dernier_traite: f64,
    /// L'instant de cette barre, en UTC.
    pub instant_traite: NaiveDateTime,
    /// L'écart, en fraction du prix servi.
    pub fraction: f64,
}

impl Perime {
    /// L'écart en points du sous-jacent.
    pub fn points(&self) -> f64 {
        self.dernier_traite - self.spot
    }

    /// Ce qu'on en dit, en une phrase, à qui lit le journal ou l'écran.
    pub fn phrase(&self) -> String {
        format!(
            "Le prix servi par la source ({:.2}) s'écarte de {:+.0} points du dernier prix \
             traité ({:.2} à {}). C'est {:.1} % — au-delà du demi pour cent toléré. Les ticks \
             d'option ne se rafraîchissent probablement plus : niveaux, zéro gamma et murs \
             sont calculés contre un prix qui n'existe plus.",
            self.spot,
            self.points(),
            self.dernier_traite,
            self.instant_traite.format("%H:%M UTC"),
            self.fraction * 100.0,
        )
    }
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

/// Le pas de temps d'un chandelier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pas {
    /// Un multiple de la minute.
    Minutes(i64),
    /// Une séance CME entière, bornée à 17 h à New York.
    Seance,
}

impl Pas {
    /// Le pas écrit comme l'écran l'envoie : `1m`, `15m`, `4h`, `1J`.
    ///
    /// Un texte inconnu ne rend pas la minute par défaut : il rend `None`, et
    /// l'appelant décide. Retomber en silence sur la minute ferait passer une
    /// faute de frappe pour un choix.
    pub fn depuis(texte: &str) -> Option<Pas> {
        let texte = texte.trim();
        if texte.eq_ignore_ascii_case("1j") || texte.eq_ignore_ascii_case("1d") {
            return Some(Pas::Seance);
        }
        let (nombre, unite) = texte.split_at(texte.len().saturating_sub(1));
        let n: i64 = nombre.parse().ok()?;
        if n <= 0 {
            return None;
        }
        match unite.to_ascii_lowercase().as_str() {
            "m" => Some(Pas::Minutes(n)),
            "h" => Some(Pas::Minutes(n * 60)),
            _ => None,
        }
    }

    /// Sa durée en secondes. Une séance vaut vingt-quatre heures : c'est le pas
    /// entre deux ouvertures, pas la durée cotée.
    pub fn secondes(self) -> i64 {
        match self {
            Pas::Minutes(n) => n.max(1) * 60,
            Pas::Seance => 86_400,
        }
    }
}

/// Le début du seau qui contient cet instant.
///
/// **Aligné sur la séance, pas sur l'époque.** Une séance de future commence à
/// 17 h à New York : découper des seaux depuis minuit UTC ferait tomber les
/// frontières en plein après-midi américain. Pour les pas d'une heure ou moins la
/// différence est nulle — la bascule tombe sur une heure ronde — mais pour quatre
/// heures et pour la journée elle décide de tout.
///
/// Et le seau se calcule sur le TEMPS, jamais sur le rang de la barre. Grouper
/// cinq barres consécutives paraît équivalent et ne l'est pas : il manque des
/// minutes dès que le marché ne traite pas, et chaque trou décalerait tous les
/// seaux suivants.
pub fn seau(instant: NaiveDateTime, pas: Pas) -> NaiveDateTime {
    let debut = gex_core::temps::debut_de_seance(instant);
    match pas {
        Pas::Seance => debut,
        Pas::Minutes(n) => {
            let n = n.max(1);
            let ecoule = (instant - debut).num_minutes();
            debut + Duration::minutes(ecoule - ecoule.rem_euclid(n))
        }
    }
}

/// Regroupe des barres d'une minute en chandeliers plus longs.
///
/// Ouverture de la première, clôture de la dernière, extrêmes des extrêmes,
/// volumes additionnés. Les barres doivent être triées — elles le sont, `recoller`
/// s'en charge.
pub fn agreger_barres(barres: &[Barre], pas: Pas) -> Vec<Barre> {
    let mut sortie: Vec<Barre> = Vec::new();
    for b in barres {
        let debut = seau(b.instant, pas);
        match sortie.last_mut() {
            Some(courant) if courant.instant == debut => {
                courant.high = courant.high.max(b.high);
                courant.low = courant.low.min(b.low);
                courant.close = b.close;
                courant.volume += b.volume;
            }
            _ => sortie.push(Barre {
                instant: debut,
                ..*b
            }),
        }
    }
    sortie
}

/// Regroupe des points de niveaux au même pas que les chandeliers.
///
/// Le **dernier** point du seau l'emporte, et non une moyenne : un zero gamma
/// moyen n'est le zero gamma d'aucun instant, et un mur moyenné tomberait entre
/// deux strikes qui n'existent pas. C'est aussi ce que fait la clôture d'un
/// chandelier — l'état à la fin du seau.
///
/// L'horodatage devient celui du seau, pour que les deux séries tombent au même
/// pixel : c'est toute la raison d'être de l'agrégation côté serveur.
pub fn agreger_niveaux(points: &[PointNiveaux], pas: Pas) -> Vec<PointNiveaux> {
    let mut sortie: Vec<PointNiveaux> = Vec::new();
    for p in points {
        let debut = seau(p.instant, pas);
        let range = PointNiveaux {
            instant: debut,
            ..*p
        };
        match sortie.last_mut() {
            Some(courant) if courant.instant == debut => *courant = range,
            _ => sortie.push(range),
        }
    }
    sortie
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
    barres
        .iter()
        .copied()
        .filter(|b| b.instant >= borne)
        .collect()
}

/// Ne garde que ce qui tombe après la borne.
pub fn elaguer_niveaux(points: &[PointNiveaux], borne: NaiveDateTime) -> Vec<PointNiveaux> {
    points
        .iter()
        .copied()
        .filter(|p| p.instant >= borne)
        .collect()
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
    let instants = barres
        .iter()
        .map(|b| b.instant.and_utc().timestamp_micros())
        .collect();
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
    let instants = points
        .iter()
        .map(|p| p.instant.and_utc().timestamp_micros())
        .collect();
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
            ("call_wall_vol", peut_etre(|p| p.call_wall_vol)),
            ("put_wall_vol", peut_etre(|p| p.put_wall_vol)),
            ("long_gamma", peut_etre(|p| p.long_gamma)),
            ("short_gamma", peut_etre(|p| p.short_gamma)),
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
fn lire_colonnes(source: &Path, noms: &[&str]) -> Result<Colonnes, ErreurReleve> {
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
            "call_wall_vol",
            "put_wall_vol",
            "long_gamma",
            "short_gamma",
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
            call_wall_vol: c[16][i],
            put_wall_vol: c[17][i],
            long_gamma: c[18][i],
            short_gamma: c[19][i],
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

    /// La panne du 3 septembre 2026, rejouée sur ses vrais chiffres.
    ///
    /// Onze heures de niveaux calculés contre 29 152 pendant que le sous-jacent
    /// traitait à 29 511. Ce test est le témoin : si un jour il cesse de passer,
    /// c'est que le garde-fou a été desserré.
    #[test]
    fn un_prix_fige_est_repere() {
        let barres = vec![
            barre("2026-09-03 20:44:00", 29_510.75),
            barre("2026-09-03 20:45:00", 29_511.00),
        ];
        let p = spot_perime(29_152.06, &barres).expect("l'écart vaut 1,2 %, il doit être vu");
        assert_eq!(p.dernier_traite, 29_511.00);
        assert!((p.points() - 358.94).abs() < 0.01, "{}", p.points());
        assert!(p.fraction > 0.012 && p.fraction < 0.013, "{}", p.fraction);
        let phrase = p.phrase();
        assert!(phrase.contains("29152.06"), "{phrase}");
        assert!(phrase.contains("20:45 UTC"), "{phrase}");
    }

    /// Une séance normale ne doit rien déclencher.
    ///
    /// Les deux chiffres ne viennent pas du même mécanisme, donc ils ne
    /// coïncident jamais exactement. Un garde-fou qui crierait sur quelques
    /// points s'apprendrait à ignorer en une séance.
    #[test]
    fn un_ecart_ordinaire_ne_declenche_pas() {
        let barres = vec![barre("2026-09-03 20:45:00", 29_500.00)];
        assert_eq!(spot_perime(29_499.25, &barres), None);
        assert_eq!(spot_perime(29_420.00, &barres), None, "0,27 % : toléré");
        assert!(
            spot_perime(29_300.00, &barres).is_some(),
            "0,68 % : au-delà du seuil"
        );
    }

    /// Sans barre, il n'y a pas de contrôle — et surtout pas un contrôle réussi.
    ///
    /// Rendre `None` ici est un choix qui se défend mal en apparence : il donne
    /// la même réponse que « tout va bien ». Mais l'appelant, lui, sait s'il a
    /// des barres ; lui faire croire à une confrontation qui n'a pas eu lieu
    /// serait pire.
    #[test]
    fn sans_barre_aucune_confrontation() {
        assert_eq!(spot_perime(29_152.06, &[]), None);
        assert_eq!(
            spot_perime(0.0, &[barre("2026-09-03 20:45:00", 29_500.0)]),
            None
        );
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
            call_wall_vol: Some(29_650.0),
            put_wall_vol: Some(29_050.0),
            put_wall_oi: Some(29_000.0),
            long_gamma: Some(29_300.0),
            short_gamma: Some(29_050.0),
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
        assert!(!faut_il_ecrire_un_point(
            Some(avant),
            instant("2026-08-26 09:15:59")
        ));
        assert!(faut_il_ecrire_un_point(
            Some(avant),
            instant("2026-08-26 09:16:01")
        ));
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
        assert!(faut_il_ecrire_un_point(
            None,
            instant("2026-08-26 09:15:02")
        ));
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

    // ---=== l'agrégation ===---

    /// Le pas se lit tel que l'écran l'envoie, et un texte inconnu se refuse
    /// plutôt que de retomber en silence sur la minute.
    #[test]
    fn le_pas_se_lit_ou_se_refuse() {
        assert_eq!(Pas::depuis("1m"), Some(Pas::Minutes(1)));
        assert_eq!(Pas::depuis("15m"), Some(Pas::Minutes(15)));
        assert_eq!(Pas::depuis("4h"), Some(Pas::Minutes(240)));
        assert_eq!(Pas::depuis("1J"), Some(Pas::Seance));
        assert_eq!(Pas::depuis("1d"), Some(Pas::Seance));
        assert_eq!(Pas::depuis("3s"), None);
        assert_eq!(Pas::depuis("0m"), None);
        assert_eq!(Pas::depuis(""), None);
    }

    /// Ouverture de la première, clôture de la dernière, extrêmes des extrêmes.
    #[test]
    fn cinq_minutes_font_un_chandelier() {
        let cinq: Vec<Barre> = (0..5)
            .map(|i| Barre {
                instant: instant(&format!("2026-08-26 14:0{i}:00")),
                open: 100.0 + i as f64,
                high: 110.0 + i as f64,
                low: 90.0 - i as f64,
                close: 105.0 + i as f64,
                volume: 10.0,
            })
            .collect();
        let a = agreger_barres(&cinq, Pas::Minutes(5));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].instant, instant("2026-08-26 14:00:00"));
        assert_eq!(a[0].open, 100.0, "l'ouverture est celle de la première");
        assert_eq!(a[0].close, 109.0, "la clôture est celle de la dernière");
        assert_eq!(a[0].high, 114.0);
        assert_eq!(a[0].low, 86.0);
        assert_eq!(a[0].volume, 50.0);
    }

    /// Le seau se calcule sur le TEMPS, jamais sur le rang. Il manque des minutes
    /// dès que le marché ne traite pas, et chaque trou décalerait sinon tous les
    /// seaux suivants.
    #[test]
    fn un_trou_ne_decale_pas_les_seaux() {
        let barres = vec![
            barre("2026-08-26 14:00:00", 100.0),
            barre("2026-08-26 14:01:00", 101.0),
            // 14:02, 14:03 et 14:04 manquent.
            barre("2026-08-26 14:05:00", 105.0),
            barre("2026-08-26 14:06:00", 106.0),
        ];
        let a = agreger_barres(&barres, Pas::Minutes(5));
        assert_eq!(a.len(), 2, "deux seaux, pas un seul de quatre barres");
        assert_eq!(a[0].instant, instant("2026-08-26 14:00:00"));
        assert_eq!(a[1].instant, instant("2026-08-26 14:05:00"));
    }

    /// À la minute, l'agrégation ne change rien du tout.
    #[test]
    fn la_minute_rend_la_serie_intacte() {
        let barres = vec![
            barre("2026-08-26 14:00:00", 100.0),
            barre("2026-08-26 14:01:00", 101.0),
        ];
        assert_eq!(agreger_barres(&barres, Pas::Minutes(1)), barres);
    }

    /// Le chandelier journalier est borné par la SÉANCE, pas par minuit UTC.
    ///
    /// 20:59 appartient encore à la séance ouverte la veille à 21 h ; 21:00 ouvre
    /// la suivante. Un découpage à minuit UTC les aurait mis dans le même seau.
    #[test]
    fn le_journalier_suit_la_seance_et_non_minuit_utc() {
        let barres = vec![
            barre("2026-08-26 20:59:00", 100.0),
            barre("2026-08-26 21:00:00", 200.0),
            barre("2026-08-27 00:00:00", 300.0),
            barre("2026-08-27 20:00:00", 400.0),
        ];
        let a = agreger_barres(&barres, Pas::Seance);
        assert_eq!(a.len(), 2, "deux séances");
        assert_eq!(a[0].instant, instant("2026-08-25 21:00:00"));
        assert_eq!(a[0].close, 100.0);
        assert_eq!(a[1].instant, instant("2026-08-26 21:00:00"));
        // `barre` pose l'ouverture cinq points sous la clôture : 195 est bien
        // l'ouverture de la barre de 21:00, donc celle de la séance.
        assert_eq!(a[1].open, 195.0, "la séance ouvre à 21 h UTC, pas à minuit");
        assert_eq!(a[1].close, 400.0);
    }

    /// Le dernier point du seau l'emporte, jamais une moyenne : un mur moyenné
    /// tomberait entre deux strikes qui n'existent pas.
    #[test]
    fn les_niveaux_gardent_le_dernier_point_du_seau() {
        let mut tot = point("2026-08-26 14:01:00", Some(29_100.0));
        tot.call_wall = Some(29_800.0);
        let mut tard = point("2026-08-26 14:04:00", Some(29_400.0));
        tard.call_wall = Some(29_900.0);
        let hors = point("2026-08-26 14:06:00", Some(29_500.0));

        let a = agreger_niveaux(&[tot, tard, hors], Pas::Minutes(5));
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].instant, instant("2026-08-26 14:00:00"));
        assert_eq!(a[0].zero_gamma, Some(29_400.0), "le dernier du seau");
        assert_eq!(a[0].call_wall, Some(29_900.0));
        assert_eq!(a[1].instant, instant("2026-08-26 14:05:00"));
    }

    /// Les deux séries doivent tomber sur les MÊMES horodatages, sans quoi
    /// l'alignement au pixel près de l'écran s'effondre.
    #[test]
    fn barres_et_niveaux_partagent_leurs_seaux() {
        let b = barre("2026-08-26 14:07:00", 100.0);
        let p = point("2026-08-26 14:07:00", Some(29_100.0));
        for pas in [
            Pas::Minutes(5),
            Pas::Minutes(15),
            Pas::Minutes(240),
            Pas::Seance,
        ] {
            assert_eq!(
                agreger_barres(&[b], pas)[0].instant,
                agreger_niveaux(&[p], pas)[0].instant,
                "seaux divergents pour {pas:?}"
            );
        }
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
        // Les deux strikes de gamma majeur sont arrivés en septembre 2026 : les
        // points d'avant n'en portent pas, et vide doit rester vide. Zéro serait
        // un prix, et l'écran tracerait une concentration de gamma au plancher.
        assert_eq!(relus[0].long_gamma, None, "colonne absente, pas un zéro");
        assert_eq!(relus[0].short_gamma, None);
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
