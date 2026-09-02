//! L'historique des relevés : une ligne de CSV par passage.
//!
//! Sans lui chaque analyse est un instantané, et les séries n'existent nulle
//! part. Le fichier est celui que `history.py` écrit depuis le début — mêmes
//! colonnes, même ordre, même format d'horodatage — parce que `validate.py` le
//! relit pour mesurer si le modèle tient. Changer une colonne rendrait illisible
//! tout ce qui a déjà été accumulé.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;

/// Les colonnes du fichier, dans l'ordre où elles y sont écrites.
///
/// Les neuf dernières décrivent la séance du sous-jacent. **Aucune source sur
/// futures ne les publie** : elles restent vides depuis qu'IB est la seule
/// source, et `validate.py` bascule alors sur la mesure à la clôture seule. Le
/// schéma les garde pour que les historiques déjà écrits restent lisibles.
///
/// L'ordre n'a plus la portée qu'il avait : [`migrer`] replace les valeurs par
/// nom, donc insérer une colonne au milieu est aussi sûr que l'ajouter à la fin.
/// Ce qui reste interdit est de **renommer** une colonne — la migration s'arrête
/// alors plutôt que de perdre ce qu'elle ne sait plus où mettre.
pub const COLONNES: [&str; 26] = [
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
    // La volatilité de la monnaie sur l'échéance la plus proche, telle que le
    // moteur la calcule. **Distincte d'`iv30`**, qui venait du CBOE et est une
    // volatilité à trente jours constants : les confondre dans une même colonne
    // mêlerait deux mesures qui ne se comparent pas. Sans elle, la vanna ne peut
    // pas être confrontée à quoi que ce soit — il n'existerait aucune variation
    // de volatilité à multiplier par elle.
    "iv_atm",
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
    /// Volatilité implicite à la monnaie sur l'échéance la plus proche.
    ///
    /// C'est elle qui rend la vanna mesurable : sans variation de volatilité
    /// d'un relevé au suivant, il n'y a rien à multiplier par la vanna, donc
    /// aucun flux de couverture à confronter au mouvement du prix.
    pub iv_atm: Option<f64>,
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
            champ(self.iv_atm),
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

/// Ce que l'écriture a dû faire au fichier avant d'y ajouter une ligne.
///
/// Rendu à l'appelant plutôt qu'imprimé ici : une bibliothèque ne parle pas à
/// l'écran, et une migration ne doit surtout pas se faire en silence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// Le fichier n'existait pas, ou était vide : l'en-tête vient d'être écrit.
    Cree,
    /// L'en-tête correspondait déjà au schéma courant.
    Aucune,
    /// Le fichier portait un en-tête plus ancien : il a été réécrit.
    Faite {
        /// Les colonnes que l'ancien en-tête n'avait pas.
        ajoutees: Vec<String>,
        /// Combien de relevés ont été reportés sous le nouvel en-tête.
        releves: usize,
        /// Où le fichier d'avant a été gardé.
        sauvegarde: PathBuf,
    },
}

/// Met l'en-tête d'un historique existant au niveau de [`COLONNES`].
///
/// **C'est ce qui manquait, et son absence était un piège muet.** L'en-tête
/// n'était écrit qu'à la création du fichier : ajouter une colonne au schéma
/// produisait des lignes à N+1 champs sous un en-tête à N. `lire_historique`
/// cherche ses colonnes par nom, donc il les aurait toutes décalées d'un cran
/// sans rien signaler — un `zero_gamma` lu dans la colonne du `call_wall`. La
/// panne la plus coûteuse qu'un fichier puisse porter est celle qui ne se voit
/// pas.
///
/// Les valeurs sont replacées **par nom**, jamais par position : un ajout en fin
/// de schéma, une insertion au milieu et un réordonnancement se traitent donc de
/// la même façon, sans code particulier pour chacun.
///
/// # Erreurs
///
/// Refuse de migrer si l'ancien en-tête porte une colonne que [`COLONNES`] ne
/// connaît plus. La reporter est impossible, la perdre serait silencieux : mieux
/// vaut s'arrêter et la nommer.
pub fn migrer(chemin: impl AsRef<Path>) -> std::io::Result<Migration> {
    let chemin = chemin.as_ref();
    if !chemin.exists() {
        return Ok(Migration::Cree);
    }
    let contenu = std::fs::read_to_string(chemin)?;
    let mut lignes = contenu.lines();
    // Un fichier de zéro octet n'a pas d'en-tête à reprendre. Le traiter comme
    // neuf vaut mieux que refuser d'écrire à cause d'un fichier vide.
    let Some(entete) = lignes.next() else {
        return Ok(Migration::Cree);
    };
    let schema = COLONNES.join(",");
    if entete == schema {
        return Ok(Migration::Aucune);
    }

    let anciennes: Vec<&str> = entete.split(',').collect();
    let inconnues: Vec<&str> = anciennes
        .iter()
        .copied()
        .filter(|c| !c.is_empty() && !COLONNES.contains(c))
        .collect();
    if !inconnues.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "l'historique porte des colonnes que le schéma ne connaît plus ({}) : \
                 migration interrompue plutôt que de les perdre",
                inconnues.join(", ")
            ),
        ));
    }

    // Où lire chaque colonne du nouveau schéma dans une ancienne ligne.
    let places: Vec<Option<usize>> = COLONNES
        .iter()
        .map(|nom| anciennes.iter().position(|a| a == nom))
        .collect();

    let mut sortie = String::with_capacity(contenu.len() + contenu.len() / 8);
    sortie.push_str(&schema);
    sortie.push('\n');
    let mut releves = 0;
    for ligne in lignes {
        if ligne.trim().is_empty() {
            continue;
        }
        let champs: Vec<&str> = ligne.split(',').collect();
        let reportee: Vec<&str> = places
            .iter()
            // Une ligne plus courte que son propre en-tête existe : une version
            // antérieure pouvait s'arrêter avant les colonnes de contexte.
            .map(|&place| place.and_then(|i| champs.get(i).copied()).unwrap_or(""))
            .collect();
        sortie.push_str(&reportee.join(","));
        sortie.push('\n');
        releves += 1;
    }

    // La copie d'avant est nommée par le nombre de colonnes qu'elle portait :
    // deux migrations successives ne se recouvrent donc jamais, et aucune
    // horloge n'est nécessaire pour les distinguer — cette crate n'y a pas accès.
    let sauvegarde = chemin.with_extension(format!("csv.avant-{}-colonnes", anciennes.len()));
    std::fs::copy(chemin, &sauvegarde)?;

    // Écriture puis renommage. Une coupure en plein milieu laisse l'historique
    // intact plutôt qu'à moitié réécrit — et un historique à moitié réécrit se
    // lirait comme une séance qui s'arrête, pas comme une panne.
    let temporaire = chemin.with_extension("csv.migration");
    std::fs::write(&temporaire, sortie)?;
    std::fs::rename(&temporaire, chemin)?;

    let ajoutees = COLONNES
        .iter()
        .zip(&places)
        .filter(|(_, place)| place.is_none())
        .map(|(nom, _)| (*nom).to_string())
        .collect();
    Ok(Migration::Faite {
        ajoutees,
        releves,
        sauvegarde,
    })
}

/// Ajoute un relevé à l'historique, après avoir mis son en-tête à niveau.
///
/// La migration est faite ici, et non laissée à l'appelant : un schéma qui
/// change alors que personne n'a pensé à migrer est exactement la situation que
/// [`migrer`] existe pour empêcher.
pub fn enregistrer(
    chemin: impl AsRef<Path>,
    ligne: &LigneHistorique,
) -> std::io::Result<Migration> {
    let chemin = chemin.as_ref();
    let migration = migrer(chemin)?;
    let mut fichier = OpenOptions::new().create(true).append(true).open(chemin)?;
    if migration == Migration::Cree {
        writeln!(fichier, "{}", COLONNES.join(","))?;
    }
    writeln!(fichier, "{}", ligne.en_csv())?;
    Ok(migration)
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
            iv_atm: Some(0.184),
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
        assert!(
            ligne()
                .en_csv()
                .starts_with("2026-08-25 20:30,NQ,30,iv,heures,")
        );
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
        let dossier = bac("une-fois");
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

    // ---=== Migration de l'en-tête ===---

    /// Un bac d'essai propre, distinct par test : la suite tourne en parallèle,
    /// et deux tests qui partagent un dossier se détruisent mutuellement.
    fn bac(nom: &str) -> std::path::PathBuf {
        let dossier = std::env::temp_dir().join(format!("gex-test-historique-{nom}"));
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        dossier
    }

    /// Écrit un historique à l'ancien format, tel qu'une version antérieure
    /// l'aurait laissé.
    fn ancien(chemin: &Path, entete: &str, lignes: &[&str]) {
        let mut contenu = entete.to_string();
        for l in lignes {
            contenu.push('\n');
            contenu.push_str(l);
        }
        contenu.push('\n');
        std::fs::write(chemin, contenu).unwrap();
    }

    fn champ_de(contenu: &str, ligne: usize, colonne: &str) -> String {
        let lignes: Vec<&str> = contenu.lines().collect();
        let i = COLONNES.iter().position(|c| *c == colonne).unwrap();
        lignes[ligne].split(',').nth(i).unwrap().to_string()
    }

    #[test]
    fn un_en_tete_deja_a_jour_ne_declenche_rien() {
        let dossier = bac("a-jour");
        let chemin = dossier.join("history.csv");
        enregistrer(&chemin, &ligne()).unwrap();
        assert_eq!(migrer(&chemin).unwrap(), Migration::Aucune);
        let _ = std::fs::remove_dir_all(&dossier);
    }

    /// **Le test qui porte tout le mécanisme.** Les valeurs suivent leur nom de
    /// colonne, jamais leur position : c'est ce qui rend un ajout, une insertion
    /// au milieu et un réordonnancement également sûrs.
    #[test]
    fn les_valeurs_suivent_leur_nom_et_non_leur_position() {
        let dossier = bac("par-nom");
        let chemin = dossier.join("history.csv");
        // Un ordre volontairement différent du schéma : si le report se faisait
        // par position, le spot atterrirait dans la colonne du ticker.
        ancien(
            &chemin,
            "ticker,timestamp,total_gex,spot",
            &["NQ,2026-08-25 20:30,-357422192,29305.75"],
        );

        let migration = migrer(&chemin).unwrap();
        let contenu = std::fs::read_to_string(&chemin).unwrap();

        assert_eq!(contenu.lines().next().unwrap(), COLONNES.join(","));
        assert_eq!(champ_de(&contenu, 1, "ticker"), "NQ");
        assert_eq!(champ_de(&contenu, 1, "timestamp"), "2026-08-25 20:30");
        assert_eq!(champ_de(&contenu, 1, "spot"), "29305.75");
        assert_eq!(champ_de(&contenu, 1, "total_gex"), "-357422192");
        // Les colonnes que l'ancien fichier n'avait pas restent vides, jamais à
        // zéro : zéro est un strike, et l'historique le relirait comme tel.
        assert_eq!(champ_de(&contenu, 1, "zero_gamma"), "");

        let Migration::Faite {
            ajoutees, releves, ..
        } = migration
        else {
            panic!("la migration aurait dû avoir lieu");
        };
        assert_eq!(releves, 1);
        assert!(ajoutees.contains(&"zero_gamma".to_string()));
        assert!(!ajoutees.contains(&"spot".to_string()));
        let _ = std::fs::remove_dir_all(&dossier);
    }

    /// Perdre une colonne serait silencieux, et le silence est précisément ce
    /// qu'on cherche à éviter ici.
    #[test]
    fn une_colonne_inconnue_arrete_la_migration() {
        let dossier = bac("inconnue");
        let chemin = dossier.join("history.csv");
        ancien(
            &chemin,
            "timestamp,ticker,spot,total_gex,une_mesure_oubliee",
            &["2026-08-25 20:30,NQ,29305.75,-357422192,42"],
        );

        let erreur = migrer(&chemin).unwrap_err();
        assert_eq!(erreur.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            erreur.to_string().contains("une_mesure_oubliee"),
            "l'erreur doit nommer la colonne en cause : {erreur}"
        );
        // Et le fichier n'a pas bougé.
        let contenu = std::fs::read_to_string(&chemin).unwrap();
        assert!(contenu.starts_with("timestamp,ticker,spot,total_gex,une_mesure_oubliee"));
        let _ = std::fs::remove_dir_all(&dossier);
    }

    /// Le fichier d'avant est irremplaçable : IB ne sert pas d'open interest
    /// historique, donc ce qui n'a pas été collecté ne se rattrape pas.
    #[test]
    fn la_migration_garde_une_copie_d_avant() {
        let dossier = bac("copie");
        let chemin = dossier.join("history.csv");
        ancien(&chemin, "timestamp,ticker", &["2026-08-25 20:30,NQ"]);
        let avant = std::fs::read_to_string(&chemin).unwrap();

        let Migration::Faite { sauvegarde, .. } = migrer(&chemin).unwrap() else {
            panic!("la migration aurait dû avoir lieu");
        };
        assert_eq!(std::fs::read_to_string(&sauvegarde).unwrap(), avant);
        // Nommée par l'ancien nombre de colonnes : deux migrations successives
        // ne se recouvrent pas.
        assert!(
            sauvegarde.to_string_lossy().ends_with("avant-2-colonnes"),
            "{}",
            sauvegarde.display()
        );
        let _ = std::fs::remove_dir_all(&dossier);
    }

    /// Une version antérieure pouvait s'arrêter avant les colonnes de contexte.
    #[test]
    fn une_ligne_plus_courte_que_son_en_tete_passe() {
        let dossier = bac("courte");
        let chemin = dossier.join("history.csv");
        ancien(
            &chemin,
            "timestamp,ticker,spot,total_gex",
            &["2026-08-25 20:30,NQ"],
        );

        migrer(&chemin).unwrap();
        let contenu = std::fs::read_to_string(&chemin).unwrap();
        assert_eq!(champ_de(&contenu, 1, "ticker"), "NQ");
        assert_eq!(champ_de(&contenu, 1, "spot"), "");
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn un_fichier_vide_est_traite_comme_neuf() {
        let dossier = bac("vide");
        let chemin = dossier.join("history.csv");
        std::fs::write(&chemin, "").unwrap();
        assert_eq!(migrer(&chemin).unwrap(), Migration::Cree);

        enregistrer(&chemin, &ligne()).unwrap();
        let contenu = std::fs::read_to_string(&chemin).unwrap();
        assert_eq!(contenu.lines().next().unwrap(), COLONNES.join(","));
        assert_eq!(contenu.lines().count(), 2);
        let _ = std::fs::remove_dir_all(&dossier);
    }

    /// Le piège que tout ceci existe pour empêcher : écrire un relevé neuf dans
    /// un fichier au vieil en-tête donnait des lignes plus longues que lui, et
    /// `lire_historique` décalait alors toutes les colonnes sans un mot.
    #[test]
    fn ecrire_dans_un_vieux_fichier_le_met_d_abord_a_niveau() {
        let dossier = bac("a-niveau");
        let chemin = dossier.join("history.csv");
        ancien(
            &chemin,
            "timestamp,ticker,spot,total_gex",
            &["2026-08-20 20:30,NQ,29000,-1000"],
        );

        enregistrer(&chemin, &ligne()).unwrap();
        let contenu = std::fs::read_to_string(&chemin).unwrap();
        let lignes: Vec<&str> = contenu.lines().collect();

        assert_eq!(lignes.len(), 3, "en-tête + l'ancien relevé + le nouveau");
        for (i, l) in lignes.iter().enumerate() {
            assert_eq!(
                l.split(',').count(),
                COLONNES.len(),
                "la ligne {i} n'a pas le compte de colonnes du schéma"
            );
        }
        assert_eq!(champ_de(&contenu, 1, "spot"), "29000");
        assert_eq!(champ_de(&contenu, 2, "spot"), "29305.75");
        let _ = std::fs::remove_dir_all(&dossier);
    }
}
