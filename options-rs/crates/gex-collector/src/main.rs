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
use std::time::{Duration, Instant};

use chrono::Utc;
use clap::Parser;
use gex_core::chaine::Chaine;
use gex_core::temps::InstantReleve;
use gex_ib::assemblage::{build_chain, fusionner};
use gex_ib::client::{ADRESSE_DEFAUT, ATTENTE_LOT, CLIENT_ID_DEFAUT, Passerelle, Valeurs};
use gex_ib::decisions::{ContratOption, avec_conid, lots, perimetre, selection_vif};
use gex_store::ecriture::{archiver, ecrire_courant};

use decisions::{MARGE_BANDE, bande_couverte, faut_il_rebalayer, faut_il_reselectionner};

/// Lignes de données entretenues par défaut.
const BUDGET_DEFAUT: usize = 90;

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

fn executer() -> Result<(), String> {
    let args = Arguments::parse();
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

    let mut socle: Option<Chaine> = None;
    let mut catalogue: Vec<(ContratOption, ibapi::contracts::Contract)> = Vec::new();
    let mut vif: Vec<(ContratOption, ibapi::contracts::Contract)> = Vec::new();
    let mut spot = 0.0_f64;
    let mut date_socle: Option<chrono::NaiveDateTime> = None;
    let mut derniere_archive: Option<Instant> = None;

    loop {
        let instant = maintenant();

        // --- le socle, une fois par journée de compensation ---
        if faut_il_rebalayer(date_socle, instant) {
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
                let repere = if spot > 0.0 {
                    spot
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
            spot = neuf.spot;
            socle = Some(neuf);
            catalogue = tous;
            date_socle = Some(instant);
            vif.clear();
        }

        let Some(socle_courant) = &socle else {
            return Err("Aucun socle : rien à entretenir.".to_string());
        };

        // --- le vif ---
        let bande = bande_couverte(&vif.iter().map(|(c, _)| c.cle.strike).collect::<Vec<_>>());
        if faut_il_reselectionner(spot, bande, MARGE_BANDE) {
            let cles: Vec<ContratOption> = catalogue.iter().map(|(c, _)| *c).collect();
            let choisis = avec_conid(&selection_vif(socle_courant, args.budget), &cles);
            vif = choisis
                .iter()
                .filter_map(|c| {
                    catalogue
                        .iter()
                        .find(|(k, _)| k.con_id == c.con_id)
                        .cloned()
                })
                .collect();
            if let Some((bas, haut)) =
                bande_couverte(&vif.iter().map(|(c, _)| c.cle.strike).collect::<Vec<_>>())
            {
                println!(
                    "\n[{}] Vif : {} contrats, bande {bas:.0} - {haut:.0}",
                    instant.format("%H:%M:%S"),
                    vif.len()
                );
            }
        }

        // --- rafraîchir, fusionner, écrire ---
        let valeurs_vif = ib
            .collecter_lot(vif.as_slice(), rafraichir)
            .map_err(|e| e.to_string())?;

        let cles_vif: Vec<ContratOption> = vif.iter().map(|(c, _)| *c).collect();
        let chaine_vif = build_chain(
            &cles_vif,
            &valeurs_vif,
            InstantReleve(maintenant()),
            Some(spot),
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
                spot = tries[tries.len() / 2];
            }

            let fondue = fusionner(socle_courant, frais, spot).map_err(|e| e.to_string())?;
            let cible = ecrire_courant(&fondue, &args.dir, &args.produit)
                .map_err(|e| e.to_string())?;
            print!(
                "\r[{}] future {spot:>10.2} — {} strikes → {}",
                maintenant().format("%H:%M:%S"),
                fondue.lignes().len(),
                cible.display()
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();

            let temps_d_archiver = derniere_archive
                .is_none_or(|avant| avant.elapsed() >= pas_archive);
            if temps_d_archiver {
                let chemin = archiver(&fondue, &args.dir, &args.produit)
                    .map_err(|e| e.to_string())?;
                derniere_archive = Some(Instant::now());
                println!("\n[{}] archive : {}", maintenant().format("%H:%M:%S"), chemin.display());
            }
        }

        // La collecte du vif a déjà duré `rafraichir` : elle EST la temporisation.
        // Dormir en plus doublerait l'intervalle entre deux écritures.
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
