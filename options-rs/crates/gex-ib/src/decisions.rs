//! Les décisions de la collecte : quoi demander, dans quel ordre, et jusqu'où.
//!
//! Elles portent tout le raisonnement et **se testent sans TWS**. C'est la même
//! coupe que côté Python, et elle n'est pas un confort : la couche réseau ne se
//! vérifie qu'avec une passerelle en marche, une authentification valide et un
//! marché ouvert. Ce qui décide doit être vérifiable sans rien de tout ça.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use chrono_tz::Tz;
use gex_core::black76::Sens;
use gex_core::chaine::Chaine;
use gex_core::temps::{EcheanceNy, InstantReleve};

/// Faute de mieux, la clôture de la séance actions à New York.
const HEURE_CLOTURE_NY: u32 = 16;

/// Ce qui identifie un contrat **sans** son `conId`.
///
/// Type distinct de [`ContratOption`] à dessein. Le vif se choisit sur la chaîne
/// au format large, où chaque ligne porte un call ET un put : le `conId` n'y
/// survit pas, il y en aurait deux par ligne. Côté Python cette absence ne se
/// voyait qu'à l'exécution — la boucle mourait au premier `reqMktData`, sur un
/// contrat sans identifiant. Ici elle est dans le type, et il faut passer par
/// [`avec_conid`] pour obtenir quelque chose de souscriptible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CleContrat {
    /// L'échéance, en heure de New York.
    pub echeance: EcheanceNy,
    /// Le prix d'exercice.
    pub strike: f64,
    /// Call ou put.
    pub sens: Sens,
}

/// Un contrat souscriptible : sa clé, et l'identifiant qu'IB lui donne.
///
/// Le `conId` suffit et vaut mieux que le reste : il désigne exactement un
/// contrat, là où symbole + échéance + strike + sens reste ambigu quand plusieurs
/// classes de cotation coexistent — et il y en a quatorze sur NQ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContratOption {
    /// L'identifiant IB.
    pub con_id: i32,
    /// Ce qui l'identifie fonctionnellement.
    pub cle: CleContrat,
}

/// Ce qui empêche de dater une échéance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErreurEcheance {
    /// L'heure servie par IB est illisible.
    HeureIllisible(String),
    /// Le fuseau servi par IB est inconnu.
    ///
    /// Doit éclater : le traiter comme New York décalerait l'échéance d'une heure
    /// ronde sans que rien ne le signale.
    FuseauInconnu(String),
}

impl std::fmt::Display for ErreurEcheance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurEcheance::HeureIllisible(h) => {
                write!(f, "Heure d'échéance illisible chez IB : {h:?}")
            }
            ErreurEcheance::FuseauInconnu(z) => write!(
                f,
                "Fuseau inconnu chez IB : {z:?}. Le prendre pour New York \
                 décalerait l'échéance d'une heure entière, en silence."
            ),
        }
    }
}

impl std::error::Error for ErreurEcheance {}

/// L'échéance datée ET horodatée, en heure de New York.
///
/// IB sert l'heure de dernière négociation dans le fuseau de la place, et les
/// classes ne s'accordent pas : 15h00 US/Central pour une hebdomadaire NQ, soit
/// 16h00 New York, mais **08h30 pour une mensuelle**, soit 9h30 — elle est réglée
/// au matin. Coder 16h en dur, la convention des actions, décalerait les
/// mensuelles de six heures et demie ; le jour de l'échéance, c'est la différence
/// entre un contrat vivant et un contrat mort.
///
/// Sans heure — IB cesse de la servir une fois l'échéance passée — on retombe sur
/// la clôture. C'est un majorant : il fait survivre un contrat quelques heures de
/// trop plutôt que de le tuer trop tôt, et le filtre d'échéance écarte de toute
/// façon ce qui est déjà échu.
pub fn instant_echeance(
    jour: NaiveDate,
    heure_derniere_negociation: Option<&str>,
    fuseau: Option<&str>,
) -> Result<EcheanceNy, ErreurEcheance> {
    let heure = heure_derniere_negociation.unwrap_or("").trim();
    if heure.is_empty() {
        return Ok(EcheanceNy(
            jour.and_hms_opt(HEURE_CLOTURE_NY, 0, 0)
                .expect("16h00 est une heure valide"),
        ));
    }

    let mut morceaux = heure.split(':');
    let mut lire = || -> Result<u32, ErreurEcheance> {
        match morceaux.next() {
            None => Ok(0),
            Some(m) => m
                .trim()
                .parse::<u32>()
                .map_err(|_| ErreurEcheance::HeureIllisible(heure.to_string())),
        }
    };
    let (h, m, s) = (lire()?, lire()?, lire()?);
    let locale = jour
        .and_time(
            NaiveTime::from_hms_opt(h, m, s)
                .ok_or_else(|| ErreurEcheance::HeureIllisible(heure.to_string()))?,
        );

    let fuseau = fuseau.unwrap_or("").trim();
    if fuseau.is_empty() {
        return Ok(EcheanceNy(locale));
    }
    let place: Tz = fuseau
        .parse()
        .map_err(|_| ErreurEcheance::FuseauInconnu(fuseau.to_string()))?;
    // On passe par l'instant absolu, puis on le réexprime à New York : c'est la
    // seule façon de traduire entre deux places dont les bascules d'heure ne
    // tombent pas forcément le même jour.
    let absolu = resoudre(place, locale);
    Ok(EcheanceNy(
        absolu.with_timezone(&chrono_tz::America::New_York).naive_local(),
    ))
}

/// Une heure locale, résolue en instant, en tranchant les deux cas du changement
/// d'heure comme le fait le moteur Python : l'heure inexistante avance, l'heure
/// vécue deux fois prend la première.
fn resoudre(place: Tz, locale: NaiveDateTime) -> chrono::DateTime<Tz> {
    use chrono::TimeZone;
    let mut essai = locale;
    for _ in 0..3 {
        match place.from_local_datetime(&essai) {
            chrono::offset::LocalResult::Single(t) => return t,
            chrono::offset::LocalResult::Ambiguous(t, _) => return t,
            chrono::offset::LocalResult::None => essai += Duration::hours(1),
        }
    }
    panic!("heure {locale} introuvable dans le fuseau {place}")
}

/// Sépare le champ d'échéance servi par IB : date, heure, fuseau.
///
/// `ibapi` consolide les trois dans `last_trade_date_or_contract_month`, sous la
/// forme `"20260827 15:00:00 US/Central"`, et laisse `last_trade_time` vide. Lire
/// le mauvais champ ne casse rien de visible : on retombe sur la clôture supposée,
/// et l'échéance se décale silencieusement de sept heures sur une mensuelle réglée
/// le matin. C'est la troisième fois que cette heure se dérobe dans ce projet.
///
/// Le format court — `"20260827"` seul — reste accepté : c'est ce que rend le
/// champ pour un contrat déjà échu.
pub fn separer_echeance(champ: &str) -> (Option<NaiveDate>, Option<&str>, Option<&str>) {
    let mut morceaux = champ.split_whitespace();
    let jour = morceaux
        .next()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok());
    (jour, morceaux.next(), morceaux.next())
}

/// Les échéances à énumérer, triées et dédoublonnées.
///
/// C'est la seule chose pour laquelle `reqSecDefOptParams` est fiable : il rend
/// l'union des échéances du sous-jacent, et cette union est exacte. Son union de
/// strikes, elle, ne l'est pas — voir [`perimetre`].
///
/// Le résultat donne le nombre d'appels à `reqContractDetails`, un par échéance.
/// Un doublon serait une requête payée pour rien, et l'ordre doit être
/// reproductible pour que deux balayages successifs se ressemblent.
pub fn echeances_utiles(
    echeances: &[NaiveDate],
    releve: InstantReleve,
    dte_max: Option<i64>,
    dte_min: i64,
) -> Vec<NaiveDate> {
    // Des jours calendaires, donc une comparaison de dates : avec l'heure de
    // collecte, une échéance du jour rendrait -1 et serait écartée à tort — or
    // c'est précisément le 0DTE.
    let aujourd_hui = releve.0.date();
    let mut gardees: Vec<NaiveDate> = echeances
        .iter()
        .copied()
        .filter(|d| {
            let jours = (*d - aujourd_hui).num_days();
            jours >= dte_min && dte_max.is_none_or(|max| jours <= max)
        })
        .collect();
    gardees.sort_unstable();
    gardees.dedup();
    gardees
}

/// Parmi les contrats **réellement cotés**, ceux à demander.
///
/// Cette fonction filtre, elle ne fabrique pas, et la distinction est le fond du
/// problème. `reqSecDefOptParams` rend l'union des strikes et l'union des
/// échéances, jamais les couples existants : leur produit cartésien est un
/// majorant, pas une chaîne. Sur NQ au 25 août 2026, quatorze classes et 3 348
/// strikes font **6 696 contrats réels** ; le produit en aurait compté 11 872,
/// soit soixante-dix-sept pour cent de trop, dont la majorité jamais cotée.
pub fn perimetre(contrats: &[ContratOption], prix: f64, plage: f64) -> Vec<ContratOption> {
    let (bas, haut) = (prix * (1.0 - plage), prix * (1.0 + plage));
    // Les bornes se calculent en flottant, donc fausses de quelques femtomètres :
    // 25 000 x 1,025 vaut 25 624,999999999996, ce qui EXCLUT le strike 25 625
    // pourtant demandé. Sans marge on perdrait le strike le plus éloigné — celui
    // qui borne le profil — et de façon imprévisible, l'erreur dépendant du prix
    // du future et changeant donc d'une séance à l'autre.
    let marge = prix.abs() * 1e-9;
    let mut retenus: Vec<ContratOption> = contrats
        .iter()
        .copied()
        .filter(|c| {
            c.cle.strike.is_finite() && c.cle.strike >= bas - marge && c.cle.strike <= haut + marge
        })
        .collect();
    trier(&mut retenus);
    retenus
}

/// L'ordre dans lequel les lots partiront : reproductible d'un balayage à l'autre.
fn trier(contrats: &mut [ContratOption]) {
    contrats.sort_by(|a, b| {
        a.cle
            .echeance
            .cmp(&b.cle.echeance)
            .then(
                a.cle
                    .strike
                    .partial_cmp(&b.cle.strike)
                    .expect("strike fini"),
            )
            .then((a.cle.sens == Sens::Put).cmp(&(b.cle.sens == Sens::Put)))
    });
}

/// Lignes de données entretenues par défaut.
///
/// Cent chez IB, et jamais le quota entier : le future en consomme une, et le
/// recyclage quelques-unes le temps que les annulations soient prises en compte.
/// Saturer le quota fait échouer les souscriptions suivantes **en silence**.
pub const BUDGET_LIGNES: usize = 90;

/// Découpe le périmètre en paquets souscriptibles d'un coup.
///
/// C'est le nombre de LOTS qui fixe le temps de balayage, pas le nombre de
/// contrats : chaque lot coûte une attente de stabilisation, et cette attente ne
/// dépend pas de sa taille. Les 6 696 contrats mesurés sur NQ font soixante-quinze
/// lots, soit trois à cinq minutes.
///
/// Un périmètre vide rend une liste vide — une échéance sans strike dans la plage
/// n'est pas une panne. Une taille nulle, elle, en est une : elle bouclerait
/// indéfiniment.
///
/// Générique parce que le découpage ne dépend pas de ce qu'on découpe : la couche
/// réseau fait voyager le contrat IB complet à côté de sa clé, et la spécialiser
/// obligerait à réécrire la même fonction deux fois.
pub fn lots<T: Clone>(contrats: &[T], taille: usize) -> Result<Vec<Vec<T>>, String> {
    if taille < 1 {
        return Err(format!("taille de lot absurde : {taille} (attendu : au moins 1)"));
    }
    Ok(contrats.chunks(taille).map(<[_]>::to_vec).collect())
}

/// Les contrats à garder souscrits en permanence, triés par `|gamma x OI|`.
///
/// Un critère géométrique — plus ou moins N strikes autour du spot — serait plus
/// simple mais dilapiderait des lignes sur des strikes sans open interest, alors
/// que le socle vient précisément de mesurer où le gamma se trouve.
///
/// Rend des [`CleContrat`] et non des contrats souscriptibles : la chaîne est au
/// format large, le `conId` n'y survit pas. Voir [`avec_conid`].
pub fn selection_vif(chaine: &Chaine, budget: usize) -> Vec<CleContrat> {
    let mut peses: Vec<(f64, CleContrat)> = Vec::new();
    for l in chaine.lignes() {
        for (sens, cote) in [(Sens::Call, &l.call), (Sens::Put, &l.put)] {
            let poids = (cote.gamma * cote.open_interest).abs();
            if poids > 0.0 {
                peses.push((
                    poids,
                    CleContrat {
                        echeance: l.echeance,
                        strike: l.strike,
                        sens,
                    },
                ));
            }
        }
    }
    // Le tri secondaire n'est pas cosmétique : à poids égaux, sans lui, deux
    // appels rendraient deux listes différentes, et le recyclage annulerait puis
    // re-souscrirait les mêmes contrats pour rien.
    peses.sort_by(|(pa, a), (pb, b)| {
        pb.partial_cmp(pa)
            .expect("poids fini")
            .then(a.echeance.cmp(&b.echeance))
            .then(a.strike.partial_cmp(&b.strike).expect("strike fini"))
            .then((a.sens == Sens::Put).cmp(&(b.sens == Sens::Put)))
    });
    peses.into_iter().take(budget).map(|(_, c)| c).collect()
}

/// Recolle le `conId` sur une sélection qui n'en porte pas.
///
/// Ce qui n'est pas au catalogue est écarté silencieusement : un contrat que le
/// socle a vu mais que l'énumération ne connaît plus n'est pas souscriptible, et
/// s'entêter ferait échouer tout le lot.
///
/// L'ordre de la sélection est préservé — c'est celui du recyclage, du plus
/// chargé en gamma au moins chargé.
pub fn avec_conid(selection: &[CleContrat], catalogue: &[ContratOption]) -> Vec<ContratOption> {
    selection
        .iter()
        .filter_map(|cle| {
            catalogue
                .iter()
                .find(|c| {
                    c.cle.echeance == cle.echeance
                        && c.cle.sens == cle.sens
                        && (c.cle.strike - cle.strike).abs() < 1e-9
                })
                .copied()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gex_core::chaine::{Cote, Ligne};

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn instant(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }
    fn ech(s: &str) -> EcheanceNy {
        EcheanceNy(instant(s))
    }
    fn contrat(con_id: i32, echeance: &str, strike: f64, sens: Sens) -> ContratOption {
        ContratOption {
            con_id,
            cle: CleContrat {
                echeance: ech(echeance),
                strike,
                sens,
            },
        }
    }

    // ---=== l'heure de l'échéance ===---

    /// Relevé chez IB le 25 août 2026 : l'hebdomadaire NQ expire à 15h00 Central.
    #[test]
    fn une_hebdomadaire_expire_a_seize_heures_new_york() {
        let e = instant_echeance(date("2026-08-26"), Some("15:00:00"), Some("US/Central")).unwrap();
        assert_eq!(e, ech("2026-08-26 16:00:00"));
    }

    /// Le piège que coder 16h en dur aurait manqué : la mensuelle est réglée le
    /// MATIN, six heures et demie plus tôt.
    #[test]
    fn une_mensuelle_est_reglee_le_matin() {
        let e = instant_echeance(date("2026-09-18"), Some("08:30:00"), Some("US/Central")).unwrap();
        assert_eq!(e, ech("2026-09-18 09:30:00"));
    }

    #[test]
    fn sans_heure_on_suppose_la_cloture() {
        for servi in [None, Some(""), Some("   ")] {
            let e = instant_echeance(date("2026-08-25"), servi, Some("US/Central")).unwrap();
            assert_eq!(e, ech("2026-08-25 16:00:00"));
        }
    }

    #[test]
    fn un_fuseau_inconnu_eclate() {
        let e = instant_echeance(date("2026-08-26"), Some("15:00:00"), Some("Mars/Olympus"));
        assert!(matches!(e, Err(ErreurEcheance::FuseauInconnu(_))));
    }

    #[test]
    fn une_heure_illisible_eclate() {
        for mauvaise in ["quinze heures", "15h00", "-1:00:00"] {
            assert!(
                matches!(
                    instant_echeance(date("2026-08-26"), Some(mauvaise), Some("US/Central")),
                    Err(ErreurEcheance::HeureIllisible(_))
                ),
                "{mauvaise} aurait dû être refusée"
            );
        }
    }

    // ---=== échéances utiles ===---

    #[test]
    fn le_champ_d_echeance_d_ib_se_separe_en_trois() {
        // Ce que TWS sert réellement, relevé le 26 août 2026.
        let (jour, heure, fuseau) = separer_echeance("20260827 15:00:00 US/Central");
        assert_eq!(jour, Some(date("2026-08-27")));
        assert_eq!(heure, Some("15:00:00"));
        assert_eq!(fuseau, Some("US/Central"));
        // Et l'échéance datée qu'on en tire
        assert_eq!(
            instant_echeance(jour.unwrap(), heure, fuseau).unwrap(),
            ech("2026-08-27 16:00:00")
        );
    }

    #[test]
    fn un_champ_d_echeance_sans_heure_reste_lisible() {
        let (jour, heure, fuseau) = separer_echeance("20260827");
        assert_eq!(jour, Some(date("2026-08-27")));
        assert_eq!((heure, fuseau), (None, None));
    }

    #[test]
    fn un_champ_d_echeance_illisible_ne_rend_aucune_date() {
        assert_eq!(separer_echeance("").0, None);
        assert_eq!(separer_echeance("bientot").0, None);
    }

    #[test]
    fn l_horizon_se_compte_en_jours_calendaires() {
        let toutes = [
            date("2026-08-25"),
            date("2026-08-26"),
            date("2026-09-18"),
            date("2026-12-18"),
        ];
        let releve = InstantReleve(instant("2026-08-25 20:30:00"));
        // Le jour même est un 0DTE, pas un contrat du passé : avec l'heure de
        // collecte, soustraire les instants l'aurait écarté à tort.
        assert_eq!(
            echeances_utiles(&toutes, releve, Some(30), 0),
            vec![date("2026-08-25"), date("2026-08-26"), date("2026-09-18")]
        );
        assert_eq!(
            echeances_utiles(&toutes, releve, Some(30), 1),
            vec![date("2026-08-26"), date("2026-09-18")]
        );
        assert_eq!(echeances_utiles(&toutes, releve, None, 0).len(), 4);
    }

    #[test]
    fn les_echeances_sont_triees_et_dedoublonnees() {
        let doublons = [date("2026-09-18"), date("2026-08-26"), date("2026-08-26")];
        let releve = InstantReleve(instant("2026-08-25 20:30:00"));
        assert_eq!(
            echeances_utiles(&doublons, releve, Some(30), 0),
            vec![date("2026-08-26"), date("2026-09-18")]
        );
    }

    // ---=== périmètre ===---

    #[test]
    fn le_perimetre_elague_ce_qui_est_hors_plage() {
        let tous: Vec<ContratOption> = [24_000.0, 25_000.0, 26_000.0, 30_000.0]
            .iter()
            .enumerate()
            .map(|(i, k)| contrat(i as i32, "2026-08-26 16:00:00", *k, Sens::Call))
            .collect();
        let retenus = perimetre(&tous, 25_000.0, 0.05);
        assert_eq!(
            retenus.iter().map(|c| c.cle.strike).collect::<Vec<_>>(),
            vec![24_000.0, 25_000.0, 26_000.0]
        );
    }

    /// La régression qui compte : 25 000 x 1,025 vaut 25 624,999999999996, donc
    /// sans marge le strike 25 625 — celui qui borne le profil — disparaît.
    #[test]
    fn la_borne_flottante_ne_perd_pas_le_dernier_strike() {
        let tous = vec![contrat(1, "2026-08-26 16:00:00", 25_625.0, Sens::Call)];
        assert_eq!(perimetre(&tous, 25_000.0, 0.025).len(), 1);
    }

    #[test]
    fn le_perimetre_est_trie_donc_reproductible() {
        let desordre = vec![
            contrat(3, "2026-08-28 16:00:00", 25_000.0, Sens::Call),
            contrat(2, "2026-08-26 16:00:00", 25_100.0, Sens::Put),
            contrat(1, "2026-08-26 16:00:00", 25_100.0, Sens::Call),
            contrat(4, "2026-08-26 16:00:00", 25_000.0, Sens::Call),
        ];
        let ordre: Vec<i32> = perimetre(&desordre, 25_000.0, 0.2)
            .iter()
            .map(|c| c.con_id)
            .collect();
        assert_eq!(ordre, vec![4, 1, 2, 3]);
    }

    // ---=== lots ===---

    #[test]
    fn les_lots_decoupent_sans_rien_perdre() {
        let tous: Vec<ContratOption> = (0..95)
            .map(|i| contrat(i, "2026-08-26 16:00:00", 25_000.0 + i as f64, Sens::Call))
            .collect();
        let paquets = lots(&tous, BUDGET_LIGNES).unwrap();
        assert_eq!(paquets.len(), 2);
        assert_eq!(paquets[0].len(), 90);
        assert_eq!(paquets[1].len(), 5);
        assert_eq!(paquets.iter().map(Vec::len).sum::<usize>(), 95);
    }

    #[test]
    fn un_perimetre_vide_ne_fait_aucun_lot() {
        let vide: [ContratOption; 0] = [];
        assert!(lots(&vide, BUDGET_LIGNES).unwrap().is_empty());
    }

    /// Une taille nulle bouclerait indéfiniment : c'est une panne, pas un cas limite.
    #[test]
    fn une_taille_de_lot_nulle_est_refusee() {
        let vide: [ContratOption; 0] = [];
        assert!(lots(&vide, 0).is_err());
    }

    // ---=== sélection du vif ===---

    fn chaine_essai() -> Chaine {
        let lignes: Vec<Ligne> = [
            // (strike, gamma call, OI call, gamma put, OI put)
            (25_000.0, 0.001, 100.0, 0.001, 50.0),
            (25_100.0, 0.002, 500.0, 0.002, 900.0),
            (25_200.0, 0.003, 10.0, 0.003, 20.0),
            (25_300.0, 0.000, 0.0, 0.000, 0.0),
        ]
        .iter()
        .map(|(k, gc, oic, gp, oip)| Ligne {
            echeance: ech("2026-08-26 16:00:00"),
            strike: *k,
            call: Cote {
                gamma: *gc,
                open_interest: *oic,
                ..Default::default()
            },
            put: Cote {
                gamma: *gp,
                open_interest: *oip,
                ..Default::default()
            },
        })
        .collect();
        Chaine::nouvelle(lignes, 25_100.0, InstantReleve(instant("2026-08-25 20:30:00"))).unwrap()
    }

    #[test]
    fn le_vif_se_choisit_par_le_gamma_pas_par_la_distance() {
        let vif = selection_vif(&chaine_essai(), 3);
        // |gamma x OI| : put 25 100 = 1,8 ; call 25 100 = 1,0 ; call 25 000 = 0,1
        assert_eq!(vif.len(), 3);
        assert_eq!(vif[0].strike, 25_100.0);
        assert_eq!(vif[0].sens, Sens::Put);
        assert_eq!(vif[1].strike, 25_100.0);
        assert_eq!(vif[1].sens, Sens::Call);
    }

    /// Un strike sans open interest ne consomme pas une ligne : le budget est le
    /// bien rare, et le dilapider sur du vide est exactement ce que le critère
    /// géométrique aurait fait.
    #[test]
    fn un_poids_nul_ne_consomme_pas_de_ligne() {
        let vif = selection_vif(&chaine_essai(), 90);
        assert_eq!(vif.len(), 6, "quatre strikes, mais un sans aucun poids");
        assert!(vif.iter().all(|c| c.strike != 25_300.0));
    }

    #[test]
    fn le_budget_est_respecte() {
        assert_eq!(selection_vif(&chaine_essai(), 2).len(), 2);
        assert!(selection_vif(&chaine_essai(), 0).is_empty());
    }

    // ---=== avec_conid ===---

    /// Le bug que le typage rend maintenant impossible à écrire : sans conId, la
    /// boucle Python mourait au premier reqMktData.
    #[test]
    fn avec_conid_recolle_les_identifiants() {
        let catalogue = vec![
            contrat(11, "2026-08-26 16:00:00", 25_100.0, Sens::Call),
            contrat(22, "2026-08-26 16:00:00", 25_100.0, Sens::Put),
        ];
        let selection = vec![
            CleContrat {
                echeance: ech("2026-08-26 16:00:00"),
                strike: 25_100.0,
                sens: Sens::Put,
            },
            CleContrat {
                echeance: ech("2026-08-26 16:00:00"),
                strike: 25_100.0,
                sens: Sens::Call,
            },
        ];
        let souscriptibles = avec_conid(&selection, &catalogue);
        // Call et put au même strike ne doivent pas se confondre, et l'ordre du
        // recyclage est préservé.
        assert_eq!(
            souscriptibles.iter().map(|c| c.con_id).collect::<Vec<_>>(),
            vec![22, 11]
        );
    }

    #[test]
    fn avec_conid_ecarte_ce_qui_n_est_pas_au_catalogue() {
        let catalogue = vec![contrat(11, "2026-08-26 16:00:00", 25_100.0, Sens::Call)];
        let selection = vec![CleContrat {
            echeance: ech("2026-08-26 16:00:00"),
            strike: 99_999.0,
            sens: Sens::Call,
        }];
        assert!(avec_conid(&selection, &catalogue).is_empty());
    }
}
