//! L'historique des relevés : une ligne de CSV par passage.
//!
//! Sans lui chaque analyse est un instantané, et les séries n'existent nulle
//! part. Le fichier est celui que `history.py` écrit depuis le début — mêmes
//! colonnes, même ordre, même format d'horodatage — parce que `validate.py` le
//! relit pour mesurer si le modèle tient. Changer une colonne rendrait illisible
//! tout ce qui a déjà été accumulé.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use chrono::NaiveDateTime;

/// Les colonnes du fichier, dans l'ordre où elles y sont écrites.
///
/// Les huit dernières décrivent la séance du sous-jacent. **Aucune source sur
/// futures ne les publie** : elles restent vides depuis qu'IB est la seule
/// source, et `validate.py` bascule alors sur la mesure à la clôture seule. Le
/// schéma les garde pour que les historiques déjà écrits restent lisibles.
pub const COLONNES: [&str; 25] = [
    "timestamp",
    "ticker",
    "dte_max",
    "source_gamma",
    "time_convention",
    "spot",
    "total_gex",
    "zero_gamma",
    "call_wall",
    "put_wall",
    "call_wall_oi",
    "put_wall_oi",
    "charm",
    "vanna",
    "strikes",
    "expiries",
    "open",
    "high",
    "low",
    "close",
    "prev_close",
    "volume",
    "dollar_volume",
    "iv30",
    "gex_sur_volume",
];

/// Un relevé, tel qu'il s'inscrit dans l'historique.
///
/// Le périmètre, la source de gamma et la convention de temps voyagent avec les
/// chiffres. Deux relevés qui n'en partagent pas ne sont **pas comparables** — un
/// `dte_max` de 7 et de 30 peuvent donner des GEX de signes opposés, et deux
/// conventions de temps déplacent le zero gamma de douze points. Sans ces
/// colonnes, rien ne permettrait de les distinguer dans le fichier.
#[derive(Debug, Clone, PartialEq)]
pub struct LigneHistorique {
    /// L'instant du relevé.
    pub instant: NaiveDateTime,
    /// Le produit.
    pub ticker: String,
    /// L'horizon d'échéance, ou `None` pour « toutes ».
    pub dte_max: Option<i64>,
    /// D'où venait le gamma.
    pub source_gamma: String,
    /// Comment le temps était mesuré.
    pub convention: String,
    /// Prix du sous-jacent.
    pub spot: f64,
    /// GEX total.
    pub gex: f64,
    /// Zero gamma retenu.
    pub zero_gamma: Option<f64>,
    /// Mur call en gamma.
    pub call_wall: Option<f64>,
    /// Mur put en gamma.
    pub put_wall: Option<f64>,
    /// Mur call en open interest.
    pub call_wall_oi: Option<f64>,
    /// Mur put en open interest.
    pub put_wall_oi: Option<f64>,
    /// Charm total.
    pub charm: f64,
    /// Vanna totale.
    pub vanna: f64,
    /// Nombre de strikes distincts.
    pub strikes: usize,
    /// Nombre d'échéances distinctes.
    pub echeances: usize,
}

/// Un flottant tel que le CSV l'attend, ou rien.
fn champ(valeur: Option<f64>) -> String {
    valeur.map_or_else(String::new, |v| v.to_string())
}

impl LigneHistorique {
    /// La ligne CSV, sans retour chariot.
    fn en_csv(&self) -> String {
        let colonnes = [
            self.instant.format("%Y-%m-%d %H:%M").to_string(),
            self.ticker.clone(),
            // « all » et non vide : une colonne vide se lirait comme une donnée
            // manquante, alors que c'est un périmètre volontairement sans borne.
            self.dte_max
                .map_or_else(|| "all".to_string(), |n| n.to_string()),
            self.source_gamma.clone(),
            self.convention.clone(),
            self.spot.to_string(),
            self.gex.to_string(),
            champ(self.zero_gamma),
            champ(self.call_wall),
            champ(self.put_wall),
            champ(self.call_wall_oi),
            champ(self.put_wall_oi),
            self.charm.to_string(),
            self.vanna.to_string(),
            self.strikes.to_string(),
            self.echeances.to_string(),
        ];
        // Les huit colonnes de contexte de séance, plus le ratio au volume :
        // vides sur futures, gardées pour que le schéma ne bouge pas.
        let vides = vec![String::new(); COLONNES.len() - colonnes.len()];
        colonnes
            .into_iter()
            .chain(vides)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Ajoute un relevé à l'historique, en créant l'en-tête si le fichier est neuf.
pub fn enregistrer(chemin: impl AsRef<Path>, ligne: &LigneHistorique) -> std::io::Result<()> {
    let chemin = chemin.as_ref();
    let neuf = !chemin.exists();
    let mut fichier = OpenOptions::new().create(true).append(true).open(chemin)?;
    if neuf {
        writeln!(fichier, "{}", COLONNES.join(","))?;
    }
    writeln!(fichier, "{}", ligne.en_csv())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ligne() -> LigneHistorique {
        LigneHistorique {
            instant: NaiveDateTime::parse_from_str("2026-08-25 20:30:16", "%Y-%m-%d %H:%M:%S")
                .unwrap(),
            ticker: "NQ".to_string(),
            dte_max: Some(30),
            source_gamma: "iv".to_string(),
            convention: "heures".to_string(),
            spot: 29_305.75,
            gex: -357_422_192.637_701_45,
            zero_gamma: None,
            call_wall: None,
            put_wall: Some(29_200.0),
            call_wall_oi: None,
            put_wall_oi: Some(28_750.0),
            charm: -15_932_881.866_839_863,
            vanna: 3_177_811.561_580_234_7,
            strikes: 142,
            echeances: 3,
        }
    }

    #[test]
    fn la_ligne_a_exactement_les_colonnes_du_schema() {
        let csv = ligne().en_csv();
        assert_eq!(
            csv.split(',').count(),
            COLONNES.len(),
            "le schéma a {} colonnes, la ligne en écrit {} : validate.py ne relirait plus rien",
            COLONNES.len(),
            csv.split(',').count()
        );
    }

    /// L'horodatage est tronqué à la minute, comme le fait Python : deux fichiers
    /// dont le format diffère ne se trient plus ensemble.
    #[test]
    fn l_horodatage_est_a_la_minute() {
        assert!(ligne().en_csv().starts_with("2026-08-25 20:30,NQ,30,iv,heures,"));
    }

    /// Un niveau absent laisse la colonne vide, jamais un zéro : zéro est un
    /// strike, et l'historique le relirait comme tel.
    #[test]
    fn un_niveau_absent_laisse_la_colonne_vide() {
        let csv = ligne().en_csv();
        let champs: Vec<&str> = csv.split(',').collect();
        let i = COLONNES.iter().position(|c| *c == "zero_gamma").unwrap();
        assert_eq!(champs[i], "");
        let j = COLONNES.iter().position(|c| *c == "put_wall").unwrap();
        assert_eq!(champs[j], "29200");
    }

    /// « all » et non vide : une colonne vide se lirait comme une donnée
    /// manquante, alors que c'est un périmètre volontairement sans borne.
    #[test]
    fn un_horizon_sans_borne_s_ecrit_all() {
        let mut l = ligne();
        l.dte_max = None;
        let champs: Vec<String> = l.en_csv().split(',').map(str::to_string).collect();
        let i = COLONNES.iter().position(|c| *c == "dte_max").unwrap();
        assert_eq!(champs[i], "all");
    }

    #[test]
    fn le_contexte_de_seance_reste_vide_sur_futures() {
        let csv = ligne().en_csv();
        let champs: Vec<&str> = csv.split(',').collect();
        for nom in [
            "open",
            "high",
            "low",
            "close",
            "prev_close",
            "volume",
            "dollar_volume",
            "iv30",
            "gex_sur_volume",
        ] {
            let i = COLONNES.iter().position(|c| *c == nom).unwrap();
            assert_eq!(champs[i], "", "colonne {nom} devrait être vide");
        }
    }

    #[test]
    fn l_en_tete_n_est_ecrit_qu_une_fois() {
        let dossier = std::env::temp_dir().join("gex-test-historique");
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        let chemin = dossier.join("history.csv");

        enregistrer(&chemin, &ligne()).unwrap();
        enregistrer(&chemin, &ligne()).unwrap();
        let contenu = std::fs::read_to_string(&chemin).unwrap();
        let lignes: Vec<&str> = contenu.lines().collect();
        assert_eq!(lignes.len(), 3, "en-tête + deux relevés");
        assert_eq!(lignes[0], COLONNES.join(","));
        assert_eq!(lignes[1], lignes[2]);
        let _ = std::fs::remove_dir_all(&dossier);
    }
}
