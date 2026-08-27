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
use gex_core::analyse::{Parametres, analyser};
use gex_core::chaine::Chaine;
use gex_core::contrat::multiplicateur;
use gex_core::temps::InstantReleve;
use gex_ib::assemblage::{build_chain, fusionner};
use gex_ib::barres::PROFONDEUR_JOURS;
use gex_ib::client::{ADRESSE_DEFAUT, ATTENTE_LOT, CLIENT_ID_DEFAUT, Passerelle, Valeurs};
use gex_ib::decisions::{ContratOption, avec_conid, lots, perimetre, selection_vif};
use gex_store::ecriture::{archiver, ecrire_courant, elaguer_archives};
use gex_store::series::{
    Barre, PointNiveaux, RETENTION_JOURS, borne_de_retention, chemin_barres, chemin_niveaux,
    a_la_minute, ecrire_barres, ecrire_niveaux, elaguer_barres, elaguer_niveaux,
    faut_il_ecrire_un_point, lire_barres, lire_niveaux, recoller,
};

use decisions::{
    MARGE_BANDE, attente_avant_reprise, bande_couverte, faut_il_rebalayer,
    faut_il_reselectionner, horizons_suivis, socle_reutilisable,
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

    /// Secondes entre deux archives horodatées. 0 pour ne rien archiver.
    ///
    /// Désactivé par défaut, et à activer en connaissance de cause : un relevé
    /// pèse 245 Ko sur NQ. Une archive par minute tient 353 Mo par jour, soit
    /// 10,6 Go sur la fenêtre de rétention ; toutes les cinq minutes, 2,1 Go.
    ///
    /// Elles sont élaguées comme les séries, mais par leur propre fenêtre :
    /// `--retention-archives`, qui suit `--retention` tant qu'on ne la pose pas.
    /// Ce qui reste est donc stable, pas cumulatif. Le coût réel est annoncé à la
    /// première archive écrite, mesuré sur elle.
    ///
    /// Elles ne servent pas au suivi de séance : les niveaux sont déjà écrits à
    /// chaque horizon. Elles servent à REJOUER une séance sous d'autres
    /// paramètres — une autre source de gamma, un autre régime de volatilité —
    /// ce que rien d'autre ne permet une fois la chaîne remplacée.
    #[arg(long, default_value = "0")]
    archiver: u64,

    /// Exiger le temps réel. Par défaut le différé, qui suffit et ne demande
    /// aucun abonnement.
    #[arg(long)]
    temps_reel: bool,

    /// Jours de barres et de niveaux conservés. 0 pour ne rien écrire.
    ///
    /// Une série qui grossit sans fin est un piège différé : le collecteur élague
    /// à chaque nouvelle journée de compensation, au même moment où il rebalaie
    /// son socle.
    #[arg(long, default_value_t = RETENTION_JOURS)]
    retention: i64,

    /// Jours d'archives conservés. Par défaut, autant que `--retention`.
    ///
    /// Séparé parce que les deux coûts n'ont rien de comparable : trente jours de
    /// séries pèsent une quinzaine de méga-octets, trente jours d'archives à la
    /// minute en pèsent dix mille. Un seul chiffre pour les deux obligeait à
    /// sacrifier l'historique des niveaux pour borner celui des archives, ou
    /// l'inverse.
    ///
    /// Le défaut suit `--retention` plutôt qu'une valeur à lui : un réglage qui
    /// s'écarterait en silence de celui qu'on vient de poser serait une surprise.
    /// Ce que ça coûte est annoncé à la première archive écrite, mesuré sur elle.
    #[arg(long, value_name = "N")]
    retention_archives: Option<i64>,

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
/// Le dernier prix connu des barres.
///
/// La clôture de la dernière barre, pas son ouverture : c'est le prix le plus
/// récent que la série porte.
fn prix_des_barres(barres: &[Barre]) -> Option<f64> {
    barres.last().map(|b| b.close).filter(|p| *p > 0.0)
}

fn balayer(
    ib: &Passerelle,
    contrats: &[(ContratOption, ibapi::contracts::Contract)],
    barres: &[Barre],
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
    let mut diagnostic: Option<String> = None;
    let mut regime: Option<ibapi::market_data::MarketDataType> = None;
    for (i, paquet) in paquets.iter().enumerate() {
        print!("\r  lot {}/{}…", i + 1, paquets.len());
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let recolte = ib
            .collecter_lot(paquet.as_slice(), attente)
            .map_err(|e| e.to_string())?;
        // Le diagnostic une seule fois, pas une par lot : soixante-quinze lots
        // qui répètent le même refus noieraient le reste du journal.
        if diagnostic.is_none()
            && let Some(dit) = recolte.diagnostic()
        {
            diagnostic = Some(dit);
        }
        regime = recolte.regime.or(regime);
        valeurs.extend(recolte.valeurs);
    }
    println!();

    // Ce qu'IB a dit pendant le balayage. Sans ça, un lot entièrement muet ne se
    // distingue pas d'un marché sans open interest — et le 10090, « il vous
    // manque l'abonnement », disparaissait avec l'erreur qui le portait.
    if let Some(dit) = &diagnostic {
        eprintln!("  IB signale : {dit}");
    }
    if let Some(r) = regime {
        println!("  données servies : {r:?}");
    }

    let cles: Vec<ContratOption> = contrats.iter().map(|(c, _)| *c).collect();
    let socle = match build_chain(&cles, &valeurs, InstantReleve(maintenant()), None) {
        Ok(chaine) => chaine,
        // IB sert le prix du sous-jacent dans chaque tick d'option — sauf quand
        // aucun contrat ne répond, ce qui arrive sur un périmètre trop serré ou
        // trop loin de la monnaie. La dernière barre le porte aussi : s'en servir
        // vaut mieux qu'abandonner un balayage qu'on vient de payer. Annoncé,
        // jamais fait en silence.
        Err(_) => {
            let secours = prix_des_barres(barres)
                .ok_or("Prix du sous-jacent introuvable : ni undPrice, ni barre.")?;
            eprintln!(
                "
  Aucun undPrice servi — prix repris de la dernière barre ({secours:.2})."
            );
            build_chain(&cles, &valeurs, InstantReleve(maintenant()), Some(secours))
                .map_err(|e| e.to_string())?
        }
    };
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
    /// Les chandeliers, relus au démarrage puis entretenus.
    barres: Vec<Barre>,
    /// La trace des niveaux, un point par minute.
    niveaux: Vec<PointNiveaux>,
    /// La minute du dernier point écrit.
    derniere_minute: Option<chrono::NaiveDateTime>,
    /// L'absence de vif a-t-elle déjà été dite ? Sans ce drapeau, le message
    /// reviendrait à chaque tour et noierait le reste du journal.
    vif_absent_signale: bool,
    /// Idem pour un marché qui ne cote pas.
    rien_ne_cote_signale: bool,
    /// Et pour un refus d'IB sur le vif.
    refus_signale: bool,
    /// Et pour des barres qui ne se rafraîchissent plus.
    barres_muettes_signale: bool,
    /// Le coût des archives a-t-il été annoncé ? Il se mesure sur la première
    /// écrite, et ne se répète pas.
    cout_archives_annonce: bool,
}

/// Une session : connexion, puis la boucle, jusqu'à la coupure.
///
/// Rend `Ok(())` quand l'arrêt a été demandé, `Err` quand la passerelle a lâché.
/// La distinction est ce qui permet à l'appelant de reprendre dans un cas et de
/// s'arrêter dans l'autre.
fn session(args: &Arguments, etat: &mut Etat, arret: &Arc<AtomicBool>) -> Result<(), String> {
    let attente = Duration::from_secs(args.attente);
    let rafraichir = Duration::from_secs(args.rafraichir);
    // Zéro veut dire « jamais » : le comparer à une durée écoulée le rendrait
    // vrai à chaque tour, donc archiverait toutes les quinze secondes.
    let pas_archive = (args.archiver > 0).then(|| Duration::from_secs(args.archiver));

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

    // Le multiplicateur du dépôt, croisé avec celui qu'IB annonce. C'est le piège
    // maison : se tromper de contrat ne produit aucune erreur, un GEX cinq fois
    // trop grand reste un nombre plausible. Les deux sources doivent s'accorder,
    // et un désaccord se dit plutôt que de se choisir en silence.
    let taille = multiplicateur(&args.produit).map_err(|e| e.to_string())?;
    if let Ok(annonce) = futur.multiplier.trim().parse::<f64>()
        && (annonce - taille).abs() > 1e-9
    {
        eprintln!(
            "Attention : IB annonce x{annonce} pour {}, le dépôt applique x{taille}.              Les niveaux enregistrés suivent le dépôt.",
            args.produit
        );
    }

    // Le vif ne survit PAS à la coupure : ses souscriptions sont mortes avec la
    // connexion, et les garder ferait croire à une bande couverte qui ne l'est
    // plus. Le socle, lui, reste : c'est de la donnée, pas un abonnement.
    etat.vif.clear();

    if args.retention > 0 {
        // La série sur disque garde l'historique long ; IB ne comble que ce qui
        // manque depuis le dernier arrêt. Le recollage écrase par horodatage,
        // parce qu'IB renvoie la dernière barre plusieurs fois pendant qu'elle se
        // forme, et que l'historique d'une reprise recouvre ce qu'on avait déjà.
        if etat.barres.is_empty() {
            etat.barres = lire_barres(&chemin_barres(&args.dir, &args.produit))
                .map_err(|e| e.to_string())?;
            etat.niveaux = lire_niveaux(&chemin_niveaux(&args.dir, &args.produit))
                .map_err(|e| e.to_string())?;
            if !etat.barres.is_empty() {
                println!(
                    "Séries relues : {} barres, {} points de niveaux.",
                    etat.barres.len(),
                    etat.niveaux.len()
                );
            }
        }
        match ib.barres(&futur, PROFONDEUR_JOURS) {
            Ok(fraiches) => {
                etat.barres = recoller(&etat.barres, &fraiches);
                println!("{} barres d'une minute.", etat.barres.len());
            }
            // Sans barres le collecteur reste utile : le GEX se calcule sans
            // elles. Le dire, et continuer, plutôt que tout arrêter.
            Err(e) => eprintln!("Barres indisponibles ({e}) — la collecte continue."),
        }
    }
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
                // Le spot connu, sinon la dernière barre. La médiane des strikes
                // servait de repère avant que les barres existent : elle centrait
                // le périmètre au milieu de la GRILLE et non du marché, si bien
                // qu'une plage serrée ne retenait que des contrats illiquides —
                // muets, donc sans undPrice, donc sans prix du tout.
                let repere = if etat.spot > 0.0 {
                    etat.spot
                } else {
                    prix_des_barres(&etat.barres).unwrap_or_else(|| {
                        let mut ks: Vec<f64> = tous.iter().map(|(c, _)| c.cle.strike).collect();
                        ks.sort_by(|a, b| a.partial_cmp(b).expect("strike fini"));
                        ks.get(ks.len() / 2).copied().unwrap_or(0.0)
                    })
                };
                let cles: Vec<ContratOption> = tous.iter().map(|(c, _)| *c).collect();
                let retenus = perimetre(&cles, repere, plage);
                tous.retain(|(c, _)| retenus.iter().any(|r| r.con_id == c.con_id));
            }

            let (neuf, valeurs) = balayer(&ib, &tous, &etat.barres, args.budget, attente)?;
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
        let recolte_vif = ib
            .collecter_lot(etat.vif.as_slice(), rafraichir)
            .map_err(|e| e.to_string())?;
        // Le vif tourne en boucle : ne dire un refus qu'une fois, sinon le
        // journal se remplit du même message toutes les quinze secondes.
        if let Some(dit) = recolte_vif.diagnostic()
            && !etat.refus_signale
        {
            eprintln!("
  IB signale sur le vif : {dit}");
            etat.refus_signale = true;
        }
        let valeurs_vif = recolte_vif.valeurs;

        let cles_vif: Vec<ContratOption> = etat.vif.iter().map(|(c, _)| *c).collect();
        let chaine_vif = build_chain(
            &cles_vif,
            &valeurs_vif,
            InstantReleve(maintenant()),
            Some(etat.spot),
        );
        // Le vif peut être vide — un périmètre où presque rien ne cote la nuit,
        // ou un socle sans gamma exploitable. Le socle, lui, PORTE de la donnée :
        // n'écrire que quand la fusion réussit laissait le collecteur tourner en
        // silence sans jamais produire de relevé, ce qui est le pire des deux
        // mondes puisque le lecteur croit simplement que rien n'a démarré.
        let sans_vif = chaine_vif.is_err();
        if sans_vif && !etat.vif_absent_signale {
            eprintln!(
                "
  Vif vide ou muet — le relevé courant reprend le socle seul.                  L'IV n'est plus rafraîchie."
            );
            etat.vif_absent_signale = true;
        }
        if !sans_vif {
            etat.vif_absent_signale = false;
        }

        {
            let frais = chaine_vif.as_ref().unwrap_or(socle_courant);
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

            // Un relevé sans le moindre open interest n'a pas de GEX : il en a
            // l'absence. Écrire un point à zéro dessinerait une ligne plate qui se
            // lirait comme une mesure — « le gamma est nul » — alors qu'elle dit
            // « le marché ne cote pas ». C'est exactement ce que la règle du dépôt
            // interdit : un GEX de zéro est un chiffre, pas une erreur.
            let cote = fondue
                .lignes()
                .iter()
                .any(|l| l.call.open_interest + l.put.open_interest > 0.0);
            if !cote && !etat.rien_ne_cote_signale {
                eprintln!(
                    "
  Aucun open interest servi — le marché ne cote pas.                      Le relevé s'écrit, la série des niveaux attend."
                );
                etat.rien_ne_cote_signale = true;
            }
            if cote {
                etat.rien_ne_cote_signale = false;
            }

            if args.retention > 0 && faut_il_ecrire_un_point(etat.derniere_minute, instant) {
                // Les barres se rafraîchissent TOUJOURS : le future se traite la
                // nuit, quand les options ne cotent pas. Les conditionner à la
                // cotation des options ferait un trou dans le graphique de prix là
                // où le marché bouge encore.
                match ib.barres(&futur, 1) {
                    Ok(fraiches) => {
                        etat.barres = recoller(&etat.barres, &fraiches);
                        if etat.barres_muettes_signale {
                            println!("  Les barres repartent.");
                            etat.barres_muettes_signale = false;
                        }
                    }
                    // Jeter cette erreur laissait le graphique de prix se figer
                    // sans un mot, pendant que le reste continuait de vivre :
                    // l'écran montrait alors un prix vieux de plusieurs minutes
                    // sous des niveaux à jour, ce qui se lit très mal.
                    Err(e) => {
                        if !etat.barres_muettes_signale {
                            eprintln!(
                                "  Barres bloquées ({e}) — le graphique de prix                                  s'arrête là, le GEX continue."
                            );
                            etat.barres_muettes_signale = true;
                        }
                    }
                }

                // UN POINT PAR HORIZON, et non un seul.
                //
                // La chaîne n'est sous la main qu'ici : une fois le relevé
                // suivant écrit, celle-ci n'existe plus nulle part. Calculer tout
                // de suite ce que chaque horizon en dit coûte quelques
                // millisecondes et une quinzaine de méga-octets par mois ; le
                // reconstituer après coup demanderait d'archiver la chaîne
                // entière chaque minute, soit 353 Mo par jour au format actuel.
                //
                // Sans cela, changer d'horizon à l'écran ne déplaçait que ce qui
                // se recalcule depuis le relevé courant : le profil et le fond
                // bougeaient, le zero gamma et les murs restaient où ils étaient.
                //
                // Le point de niveaux n'existe que si quelque chose cote.
                if cote {
                    for horizon in horizons_suivis(args.dte_max) {
                        let a = match analyser(
                            &fondue,
                            &Parametres {
                                taille_contrat: taille,
                                dte_max: Some(horizon),
                                dte_min: args.dte_min,
                                ..Default::default()
                            },
                        ) {
                            Ok(a) => a,
                            // Un horizon vide n'est pas une panne : hors séance,
                            // « 0DTE » ne contient rien, l'échéance du jour étant
                            // déjà réglée. On saute celui-là, les autres passent.
                            Err(_) => continue,
                        };
                        etat.niveaux.push(PointNiveaux {
                            instant: a_la_minute(instant),
                            spot: a.spot,
                            zero_gamma: a.zero_gamma,
                            gex: a.gex,
                            charm: a.charm,
                            vanna: a.vanna,
                            call_wall: a.murs.call,
                            put_wall: a.murs.put,
                            call_wall_oi: a.murs.call_oi,
                            put_wall_oi: a.murs.put_oi,
                            iv_atm: a.iv_atm,
                            skew: a.skew,
                            max_pain: a.max_pain,
                            delta: Some(a.delta),
                            vega: Some(a.vega),
                            theta: Some(a.theta),
                            // C'est cette colonne qui distingue les points d'une
                            // même minute. Sans elle, ils se liraient comme des
                            // mesures contradictoires du même instant.
                            dte_max: Some(horizon as f64),
                        });
                    }
                }
                // L'élagage suit la même horloge que le socle : une borne
                // glissante, appliquée quand on écrit, sans mécanisme de plus.
                let borne = borne_de_retention(instant, args.retention);
                etat.barres = elaguer_barres(&etat.barres, borne);
                etat.niveaux = elaguer_niveaux(&etat.niveaux, borne);

                // Les archives suivent la MÊME fenêtre glissante que les séries.
                // Sans cela, `--archiver` était un piège différé : un relevé de
                // 245 Ko à chaque cadence, et rien pour l'effacer. À la minute,
                // 353 Mo par jour qui s'ajoutent indéfiniment ; avec la fenêtre,
                // 10,6 Go stables. Ce qui s'efface se dit, comme le reste.
                if pas_archive.is_some() {
                    let jours = args.retention_archives.unwrap_or(args.retention);
                    let borne = borne_de_retention(instant, jours);
                    match elaguer_archives(&args.dir, &args.produit, borne) {
                        Ok(0) => {}
                        Ok(n) => println!("  {n} archive(s) hors des {jours} jours effacée(s)."),
                        // Une archive qui résiste n'arrête pas la collecte, mais
                        // ne part pas non plus en silence : le dossier grossirait
                        // sans que rien ne l'explique.
                        Err(e) => eprintln!("  Élagage des archives impossible ({e}) — le dossier va grossir."),
                    }
                }

                ecrire_barres(&etat.barres, &chemin_barres(&args.dir, &args.produit))
                    .map_err(|e| e.to_string())?;
                ecrire_niveaux(&etat.niveaux, &chemin_niveaux(&args.dir, &args.produit))
                    .map_err(|e| e.to_string())?;
                etat.derniere_minute = Some(a_la_minute(instant));
            }

            let temps_d_archiver = pas_archive.is_some_and(|pas| {
                etat.derniere_archive
                    .is_none_or(|avant| avant.elapsed() >= pas)
            });
            if temps_d_archiver {
                let chemin = archiver(&fondue, &args.dir, &args.produit)
                    .map_err(|e| e.to_string())?;
                etat.derniere_archive = Some(Instant::now());
                println!("\n[{}] archive : {}", maintenant().format("%H:%M:%S"), chemin.display());

                // Ce que les archives vont coûter, dit une fois, et MESURÉ sur
                // celle qu'on vient d'écrire plutôt qu'estimé sur une moyenne.
                // Le chiffre dépend du produit — une chaîne NQ pèse dix fois une
                // chaîne peu cotée — et personne ne devrait avoir à le découvrir
                // en regardant son disque se remplir.
                if !etat.cout_archives_annonce
                    && let Ok(taille) = std::fs::metadata(&chemin).map(|m| m.len())
                {
                    etat.cout_archives_annonce = true;
                    let jours = args.retention_archives.unwrap_or(args.retention);
                    let par_jour = taille as f64 * 86_400.0 / args.archiver.max(1) as f64;
                    println!(
                        "  {} Ko l'archive, une toutes les {} s : compter {:.1} Go sur {jours} jours.\n\
                           --retention-archives borne les archives sans toucher aux séries.",
                        taille / 1024,
                        args.archiver,
                        par_jour * jours as f64 / 1e9,
                    );
                }
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
