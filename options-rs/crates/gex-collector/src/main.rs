//! Le collecteur : socle une fois par jour, vif entretenu, relevé réécrit.
//!
//! Séparé du lecteur, et pas par goût du découpage. NQ se traite près de
//! vingt-quatre heures sur vingt-quatre, et TWS comme Gateway se redémarrent de
//! force **une fois par jour** — c'est ainsi qu'IB recharge les définitions de
//! contrats. Un unique processus qui ferait acquisition et calcul perdrait son
//! socle à chaque coupure. Ici le collecteur encaisse et réécrit son relevé ; le
//! lecteur ouvre un fichier, et se moque de savoir si le collecteur tourne.
//!
//! **Lecture seule.** Le collecteur énumère des contrats et souscrit à des
//! cotations. Aucun ordre n'est passé nulle part.

#![forbid(unsafe_code)]

mod decisions;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use clap::Parser;
use gex_core::chaine::Chaine;
use gex_core::temps::InstantReleve;
use gex_ib::assemblage::{build_chain, fusionner};
use gex_ib::client::{ADRESSE_DEFAUT, ATTENTE_LOT, CLIENT_ID_DEFAUT, Passerelle, Valeurs};
use gex_ib::decisions::{ContratOption, avec_conid, lots, perimetre, selection_vif};
use gex_store::ecriture::{archiver, ecrire_courant};

use decisions::{
    MARGE_BANDE, attente_avant_reprise, bande_couverte, faut_il_rebalayer,
    faut_il_reselectionner, socle_reutilisable,
};

/// Lignes de données entretenues par défaut.
const BUDGET_DEFAUT: usize = 90;

/// Durée au-delà de laquelle une session compte comme ayant tenu.
///
/// En deçà, la connexion n'a jamais vraiment fonctionné et l'attente doit
/// continuer de croître ; au-delà, la coupure est un événement isolé et le
/// compteur repart de zéro.
const SESSION_SAINE: Duration = Duration::from_secs(120);

#[derive(Parser, Debug)]
#[command(
    name = "gex-collector",
    about = "Collecteur Interactive Brokers : socle quotidien, vif entretenu"
)]
struct Arguments {
    /// Produit CME : NQ, ES...
    #[arg(default_value = "NQ")]
    produit: String,

    /// Place de cotation.
    #[arg(long, default_value = "CME")]
    exchange: String,

    /// Demi-plage de strikes autour du spot. Par défaut : tout prendre.
    ///
    /// Filtrer n'économise pas grand-chose — de 20 % à 5 %, vingt-sept pour cent
    /// des contrats — alors qu'une donnée non collectée est perdue pour toujours,
    /// et que le lecteur filtre déjà au calcul.
    #[arg(long)]
    range: Option<f64>,

    /// Horizon en jours. 7 démarre bien plus vite que 30.
    #[arg(long, value_name = "N", default_value = "30")]
    dte_max: i64,

    /// Exclure les échéances à moins de N jours. 1 écarte les 0DTE.
    #[arg(long, value_name = "N", default_value = "0")]
    dte_min: i64,

    /// Lignes de données entretenues. 100 au maximum sans Quote Booster.
    #[arg(long, default_value_t = BUDGET_DEFAUT)]
    budget: usize,

    /// Délai de garde par lot, en secondes.
    #[arg(long, default_value_t = ATTENTE_LOT.as_secs())]
    attente: u64,

    /// Secondes entre deux écritures du relevé courant.
    #[arg(long, default_value = "15")]
    rafraichir: u64,

    /// Secondes entre deux archives horodatées.
    #[arg(long, default_value = "900")]
    archiver: u64,

    /// Exiger le temps réel. Par défaut le différé, qui suffit et ne demande
    /// aucun abonnement.
    #[arg(long)]
    temps_reel: bool,

    /// Dossier des relevés.
    #[arg(long, default_value = "snapshots")]
    dir: PathBuf,

    /// Adresse de TWS ou Gateway.
    #[arg(long, default_value = ADRESSE_DEFAUT)]
    adresse: String,

    /// Identifiant client.
    #[arg(long, default_value_t = CLIENT_ID_DEFAUT)]
    client_id: i32,
}

/// L'instant courant, en UTC et sans fuseau attaché.
///
/// Le collecteur est le seul à lire l'horloge — `gex-core` en est incapable par
/// construction. C'est ce qui garde tout le calcul reproductible.
fn maintenant() -> chrono::NaiveDateTime {
    Utc::now().naive_utc()
}

/// Balaie tout le périmètre par lots, et en fait le socle.
fn balayer(
    ib: &Passerelle,
    contrats: &[(ContratOption, ibapi::contracts::Contract)],
    budget: usize,
    attente: Duration,
) -> Result<(Chaine, HashMap<i32, Valeurs>), String> {
    let paquets = lots(contrats, budget)?;
    println!(
        "  {} contrats, {} lots — compter {:.0} min",
        contrats.len(),
        paquets.len(),
        paquets.len() as f64 * (attente.as_secs_f64() + 2.6) / 60.0
    );

    let mut valeurs: HashMap<i32, Valeurs> = HashMap::new();
    for (i, paquet) in paquets.iter().enumerate() {
        print!("\r  lot {}/{}…", i + 1, paquets.len());
        use std::io::Write;
        let _ = std::io::stdout().flush();
        valeurs.extend(
            ib.collecter_lot(paquet.as_slice(), attente)
                .map_err(|e| e.to_string())?,
        );
    }
    println!();

    let cles: Vec<ContratOption> = contrats.iter().map(|(c, _)| *c).collect();
    let socle = build_chain(&cles, &valeurs, InstantReleve(maintenant()), None)
        .map_err(|e| e.to_string())?;
    Ok((socle, valeurs))
}

/// Ce que le collecteur garde d'une session à l'autre.
///
/// Sorti de la boucle pour survivre à la coupure. C'est tout l'intérêt de traiter
/// le redémarrage quotidien de TWS comme un événement normal : le socle est déjà
/// là, l'open interest qu'il porte ne bougera pas avant la publication du soir, et
/// le rebalayer coûterait trois à cinq minutes pour relire les mêmes chiffres.
#[derive(Default)]
struct Etat {
    socle: Option<Chaine>,
    catalogue: Vec<(ContratOption, ibapi::contracts::Contract)>,
    vif: Vec<(ContratOption, ibapi::contracts::Contract)>,
    spot: f64,
    date_socle: Option<chrono::NaiveDateTime>,
    derniere_archive: Option<Instant>,
}

/// Une session : connexion, puis la boucle, jusqu'à la coupure.
///
/// Rend `Ok(())` quand l'arrêt a été demandé, `Err` quand la passerelle a lâché.
/// La distinction est ce qui permet à l'appelant de reprendre dans un cas et de
/// s'arrêter dans l'autre.
fn session(args: &Arguments, etat: &mut Etat, arret: &Arc<AtomicBool>) -> Result<(), String> {
    let attente = Duration::from_secs(args.attente);
    let rafraichir = Duration::from_secs(args.rafraichir);
    let pas_archive = Duration::from_secs(args.archiver);

    let ib = Passerelle::connecter(&args.adresse, args.client_id, !args.temps_reel)
        .map_err(|e| e.to_string())?;
    println!(
        "Connecté à {} — données {}",
        args.adresse,
        if ib.differe { "DIFFÉRÉES (~15 min)" } else { "temps réel" }
    );

    let futur = ib
        .front_month(&args.produit, &args.exchange)
        .map_err(|e| e.to_string())?;
    println!(
        "Future {} — multiplicateur x{}",
        futur.local_symbol, futur.multiplier
    );

    // Le vif ne survit PAS à la coupure : ses souscriptions sont mortes avec la
    // connexion, et les garder ferait croire à une bande couverte qui ne l'est
    // plus. Le socle, lui, reste : c'est de la donnée, pas un abonnement.
    etat.vif.clear();
    if etat.socle.is_some() && socle_reutilisable(etat.date_socle, maintenant()) {
        println!("Socle du jour conservé — pas de rebalayage.");
    }

    loop {
        if arret.load(Ordering::Relaxed) {
            return Ok(());
        }
        // Avant tout : la passerelle répond-elle ? Une souscription lancée sur une
        // connexion morte ne rend pas d'erreur, elle BLOQUE — le collecteur reste
        // alors vivant, muet, et n'écrit plus rien. Mesuré en coupant TWS pendant
        // qu'il tournait : sans ce test, le processus se fige sans un mot.
        if !ib.vivante() {
            return Err("la passerelle ne répond plus".to_string());
        }
        let instant = maintenant();

        // --- le socle, une fois par journée de compensation ---
        if faut_il_rebalayer(etat.date_socle, instant) {
            println!("\n[{}] Balayage du socle…", instant.format("%H:%M:%S"));
            let mut tous = ib
                .enumerer(
                    &futur,
                    &args.exchange,
                    InstantReleve(instant),
                    Some(args.dte_max),
                    args.dte_min,
                    true,
                )
                .map_err(|e| e.to_string())?;

            if let Some(plage) = args.range {
                // Le repère est le spot connu, ou la médiane des strikes au tout
                // premier balayage — aucun tick n'existe encore à ce moment-là.
                let repere = if etat.spot > 0.0 {
                    etat.spot
                } else {
                    let mut ks: Vec<f64> = tous.iter().map(|(c, _)| c.cle.strike).collect();
                    ks.sort_by(|a, b| a.partial_cmp(b).expect("strike fini"));
                    ks.get(ks.len() / 2).copied().unwrap_or(0.0)
                };
                let cles: Vec<ContratOption> = tous.iter().map(|(c, _)| *c).collect();
                let retenus = perimetre(&cles, repere, plage);
                tous.retain(|(c, _)| retenus.iter().any(|r| r.con_id == c.con_id));
            }

            let (neuf, valeurs) = balayer(&ib, &tous, args.budget, attente)?;
            let muets = tous.len() - valeurs.len();
            println!(
                "  socle : {} strikes, future {:.2} ({muets} contrats sans réponse)",
                neuf.lignes().len(),
                neuf.spot
            );
            etat.spot = neuf.spot;
            etat.socle = Some(neuf);
            etat.catalogue = tous;
            etat.date_socle = Some(instant);
            etat.vif.clear();
        }

        let Some(socle_courant) = &etat.socle else {
            return Err("Aucun socle : rien à entretenir.".to_string());
        };

        // --- le vif ---
        let bande = bande_couverte(&etat.vif.iter().map(|(c, _)| c.cle.strike).collect::<Vec<_>>());
        if faut_il_reselectionner(etat.spot, bande, MARGE_BANDE) {
            let cles: Vec<ContratOption> = etat.catalogue.iter().map(|(c, _)| *c).collect();
            let choisis = avec_conid(&selection_vif(socle_courant, args.budget), &cles);
            etat.vif = choisis
                .iter()
                .filter_map(|c| {
                    etat.catalogue
                        .iter()
                        .find(|(k, _)| k.con_id == c.con_id)
                        .cloned()
                })
                .collect();
            if let Some((bas, haut)) =
                bande_couverte(&etat.vif.iter().map(|(c, _)| c.cle.strike).collect::<Vec<_>>())
            {
                println!(
                    "\n[{}] Vif : {} contrats, bande {bas:.0} - {haut:.0}",
                    instant.format("%H:%M:%S"),
                    etat.vif.len()
                );
            }
        }

        // --- rafraîchir, fusionner, écrire ---
        let valeurs_vif = ib
            .collecter_lot(etat.vif.as_slice(), rafraichir)
            .map_err(|e| e.to_string())?;

        let cles_vif: Vec<ContratOption> = etat.vif.iter().map(|(c, _)| *c).collect();
        let chaine_vif = build_chain(
            &cles_vif,
            &valeurs_vif,
            InstantReleve(maintenant()),
            Some(etat.spot),
        );
        if let Ok(frais) = &chaine_vif {
            // Le prix du future vient du vif : c'est la seule chose qui bouge
            // vraiment en séance, avec l'IV.
            let vus: Vec<f64> = valeurs_vif
                .values()
                .filter_map(|v| v.sous_jacent)
                .filter(|p| *p > 0.0)
                .collect();
            if !vus.is_empty() {
                let mut tries = vus;
                tries.sort_by(|a, b| a.partial_cmp(b).expect("prix fini"));
                etat.spot = tries[tries.len() / 2];
            }

            let fondue = fusionner(socle_courant, frais, etat.spot).map_err(|e| e.to_string())?;
            let cible = ecrire_courant(&fondue, &args.dir, &args.produit)
                .map_err(|e| e.to_string())?;
            print!(
                "\r[{}] future {:>10.2} — {} strikes → {}",
                maintenant().format("%H:%M:%S"),
                etat.spot,
                fondue.lignes().len(),
                cible.display()
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();

            let temps_d_archiver = etat
                .derniere_archive
                .is_none_or(|avant| avant.elapsed() >= pas_archive);
            if temps_d_archiver {
                let chemin = archiver(&fondue, &args.dir, &args.produit)
                    .map_err(|e| e.to_string())?;
                etat.derniere_archive = Some(Instant::now());
                println!("\n[{}] archive : {}", maintenant().format("%H:%M:%S"), chemin.display());
            }
        }

        // La collecte du vif a déjà duré `rafraichir` : elle EST la temporisation.
        // Dormir en plus doublerait l'intervalle entre deux écritures.
    }
}

fn executer() -> Result<(), String> {
    let args = Arguments::parse();

    // Ctrl+C ne tue plus le processus : il lève ce drapeau, la boucle le voit au
    // tour suivant, et la connexion se ferme proprement. Un arrêt brutal laisserait
    // des souscriptions ouvertes côté IB, qui consomment le quota de cent lignes
    // jusqu'à ce que TWS les recycle de lui-même.
    let arret = Arc::new(AtomicBool::new(false));
    let drapeau = Arc::clone(&arret);
    ctrlc::set_handler(move || {
        if drapeau.swap(true, Ordering::Relaxed) {
            // Deuxième Ctrl+C : l'utilisateur insiste, on ne le fait pas attendre
            // la fin du lot en cours.
            eprintln!("
Arrêt immédiat.");
            std::process::exit(130);
        }
        eprintln!("
Arrêt demandé — fin du cycle en cours…");
    })
    .map_err(|e| format!("impossible d'installer l'arrêt propre : {e}"))?;

    let mut etat = Etat::default();
    let mut tentative = 0u32;

    loop {
        let depart = Instant::now();
        match session(&args, &mut etat, &arret) {
            Ok(()) => {
                println!("
Déconnecté.");
                return Ok(());
            }
            // Une coupure n'est PAS une panne : TWS se redémarre de force une
            // fois par jour, et l'authentification expire le dimanche à 1 h
            // heure de New York. Le collecteur doit encaisser les deux.
            Err(motif) => {
                if arret.load(Ordering::Relaxed) {
                    println!("
Déconnecté.");
                    return Ok(());
                }
                // Une session qui a tenu n'est pas un échec de connexion : sans
                // cette remise à zéro, trois coupures dans la journée feraient
                // attendre cinq minutes à la troisième, alors que chacune s'est
                // rétablie du premier coup.
                if depart.elapsed() >= SESSION_SAINE {
                    tentative = 0;
                }
                let pause = attente_avant_reprise(tentative);
                tentative += 1;
                eprintln!(
                    "
[{}] Connexion perdue ({motif}) — reprise dans {}s (tentative {tentative}).",
                    maintenant().format("%H:%M:%S"),
                    pause.as_secs()
                );
                // Un sommeil d'un bloc ignorerait le Ctrl+C pendant cinq minutes.
                let reveil = Instant::now() + pause;
                while Instant::now() < reveil && !arret.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(200).min(pause));
                }
                if arret.load(Ordering::Relaxed) {
                    return Ok(());
                }
            }
        }
    }
}

fn main() -> ExitCode {
    match executer() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("\nErreur : {message}");
            ExitCode::FAILURE
        }
    }
}
