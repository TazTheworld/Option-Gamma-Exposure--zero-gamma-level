//! Le lecteur : ouvre un relevé et en tire les chiffres de la séance.
//!
//! Il ne va jamais chercher de données. Le collecteur écrit un fichier, le
//! lecteur le lit — c'est cette séparation qui le laisse tourner pendant que le
//! collecteur encaisse le redémarrage quotidien d'IB, et qui fait qu'une séance
//! passée se rejoue avec exactement le même code qu'une séance vivante.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use gex_core::analyse::{Analyse, Parametres, RegimeVol, SourceGamma, analyser};
use gex_core::contrat::multiplicateur;
use gex_core::temps::{Convention, InstantReleve};
use gex_store::historique::{LigneHistorique, enregistrer};
use gex_store::{chemin_courant, lire_releve};

#[derive(Parser, Debug)]
#[command(
    name = "gex",
    about = "Gamma exposure et zero gamma level sur options de futures (Interactive Brokers)"
)]
struct Arguments {
    /// Produit CME : NQ, ES...
    #[arg(default_value = "NQ")]
    produit: String,

    /// Rejouer un relevé archivé au lieu du relevé courant.
    #[arg(long, value_name = "FICHIER")]
    replay: Option<PathBuf>,

    /// Dossier des relevés.
    #[arg(long, default_value = "snapshots")]
    dir: PathBuf,

    /// Horizon d'échéance en jours. Omettre `--dte-max` garde toute la chaîne.
    #[arg(long, value_name = "N", default_value = "30")]
    dte_max: i64,

    /// Garder toute la chaîne, sans filtre d'horizon.
    #[arg(long, conflicts_with = "dte_max")]
    toutes_echeances: bool,

    /// Exclure les échéances à moins de N jours.
    #[arg(long, value_name = "N", default_value = "0")]
    dte_min: i64,

    /// Demi-plage du profil autour du spot. 0.2 = +/-20 %.
    #[arg(long, default_value = "0.2")]
    range: f64,

    /// Multiplicateur du contrat. Par défaut celui du produit.
    #[arg(long)]
    contract_size: Option<f64>,

    /// D'où vient le gamma du GEX par strike.
    #[arg(long, value_enum, default_value_t = SourceCli::Iv)]
    gamma_source: SourceCli,

    /// Comment mesurer le temps restant.
    #[arg(long, value_enum, default_value_t = ConventionCli::Heures)]
    time_convention: ConventionCli,

    /// Ce que devient la volatilité quand le spot bouge, dans le profil.
    #[arg(long, value_enum, default_value_t = RegimeCli::StickyStrike)]
    vol_regime: RegimeCli,

    /// Relire le relevé en boucle : 30s, 5m, 1h. Interdit avec --replay.
    #[arg(long, value_name = "INTERVALLE")]
    watch: Option<String>,

    /// Durée totale du suivi.
    #[arg(long, value_name = "DUREE", default_value = "6h")]
    watch_duration: String,

    /// Fichier d'historique.
    #[arg(long, default_value = "history.csv")]
    history: PathBuf,

    /// Ne pas enregistrer ce relevé dans l'historique.
    #[arg(long)]
    no_history: bool,
}

/// « 6h », « 90m », « 3600 » -> secondes.
fn duree(texte: &str) -> Result<u64, String> {
    let brut = texte.trim().to_ascii_lowercase();
    let (nombre, facteur) = match brut.chars().last() {
        Some('h') => (&brut[..brut.len() - 1], 3600.0),
        Some('m') => (&brut[..brut.len() - 1], 60.0),
        Some('s') => (&brut[..brut.len() - 1], 1.0),
        _ => (brut.as_str(), 1.0),
    };
    nombre
        .trim()
        .parse::<f64>()
        .map(|v| (v * facteur) as u64)
        .map_err(|_| format!("durée invalide : {texte}"))
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum SourceCli {
    Iv,
    Published,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ConventionCli {
    Heures,
    Bourse,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum RegimeCli {
    StickyStrike,
    StickyMoneyness,
}

/// Un nombre avec séparateurs de milliers, à `decimales` chiffres.
///
/// Écrit à la main : la bibliothèque standard ne sait pas grouper les milliers,
/// et une dépendance de plus pour dix lignes de formatage serait mal placée.
fn groupe(valeur: f64, decimales: usize) -> String {
    let brut = format!("{:.*}", decimales, valeur.abs());
    let (entier, reste) = brut.split_once('.').unwrap_or((brut.as_str(), ""));
    let mut sortie = String::new();
    for (i, c) in entier.chars().enumerate() {
        if i > 0 && (entier.len() - i) % 3 == 0 {
            sortie.push(',');
        }
        sortie.push(c);
    }
    if !reste.is_empty() {
        sortie.push('.');
        sortie.push_str(reste);
    }
    if valeur < 0.0 { format!("-{sortie}") } else { sortie }
}

/// L'unité d'affichage, selon l'ordre de grandeur.
fn echelle(valeurs: &[f64]) -> (f64, &'static str) {
    let pic = valeurs
        .iter()
        .filter(|v| v.is_finite())
        .fold(0.0_f64, |acc, v| acc.max(v.abs()));
    if pic >= 1e9 {
        (1e9, "milliards")
    } else {
        (1e6, "millions")
    }
}

/// Comme [`groupe`], mais le signe est toujours ecrit.
///
/// `{:+}` ne s'applique qu'aux nombres, pas aux chaines : sans cette fonction le
/// charm et la vanna perdaient leur `+` des qu'ils etaient positifs, et un flux
/// de couverture dont le SENS est toute l'information se lisait a l'envers d'un
/// coup d'oeil.
fn groupe_signe(valeur: f64, decimales: usize) -> String {
    let texte = groupe(valeur, decimales);
    if valeur >= 0.0 {
        format!("+{texte}")
    } else {
        texte
    }
}

fn optionnel(valeur: Option<f64>, decimales: usize) -> String {
    valeur.map_or_else(|| "n/a".to_string(), |v| groupe(v, decimales))
}

/// Le rapport de séance, dans la forme qu'avait le moteur Python.
fn afficher(a: &Analyse, produit: &str, args: &Arguments, dte_max: Option<i64>) {
    let dec = if a.spot < 10.0 { 4 } else { 2 };
    let echeances = a
        .lignes
        .iter()
        .map(|l| l.ligne.echeance)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let strikes = a.par_strike.len();
    let horizon = dte_max.map_or_else(|| "toutes échéances".to_string(), |n| format!("<= {n}j"));

    println!(
        "{produit} | sous-jacent {} | {strikes} strikes / {echeances} échéances ({horizon}) \
         | {} | contrat x{} | gamma {} | T {} | vol {}",
        groupe(a.spot, dec),
        a.releve.0.format("%Y-%m-%d %H:%M"),
        groupe(a.taille_contrat, 0),
        match args.gamma_source {
            SourceCli::Iv => "iv",
            SourceCli::Published => "published",
        },
        match args.time_convention {
            ConventionCli::Heures => "heures",
            ConventionCli::Bourse => "bourse",
        },
        match args.vol_regime {
            RegimeCli::StickyStrike => "sticky-strike",
            RegimeCli::StickyMoneyness => "sticky-moneyness",
        },
    );

    let (ech, unite) = echelle(&[a.gex]);
    let (gech, gunite) = echelle(&[a.charm, a.vanna]);
    println!(
        "Total GEX  : {} {unite} $ / mouvement de 1%",
        groupe(a.gex / ech, 2)
    );
    println!("Zero Gamma : {}", optionnel(a.zero_gamma, dec));
    println!(
        "Call Wall  : {:>12} (gamma)   {:>12} (open interest)",
        optionnel(a.murs.call, dec),
        optionnel(a.murs.call_oi, dec)
    );
    println!(
        "Put Wall   : {:>12} (gamma)   {:>12} (open interest)",
        optionnel(a.murs.put, dec),
        optionnel(a.murs.put_oi, dec)
    );
    // Charm : delta que les dealers doivent racheter (négatif) ou revendre
    // (positif) pour chaque jour qui passe, à prix inchangé.
    println!(
        "Charm      : {} {gunite} $ de delta / jour",
        groupe_signe(a.charm / gech, 2)
    );
    println!(
        "Vanna      : {} {gunite} $ de delta / point de vol",
        groupe_signe(a.vanna / gech, 2)
    );

    avertir(a, dec);
}

/// Les diagnostics qui décident si les chiffres ci-dessus sont lisibles.
///
/// Le dépôt a une règle : rien en silence. Un mode dégradé, une approximation ou
/// un désaccord entre deux estimateurs s'annonce à l'écran, faute de quoi le
/// lecteur prend pour argent comptant un chiffre que le calcul sait douteux.
fn avertir(a: &Analyse, dec: usize) {
    if a.croisements.is_empty() {
        println!(
            "\nAttention : pas de changement de signe du gamma dans la plage analysée \
             ({} - {}) — élargis avec --range, ou allonge l'horizon avec --dte-max.",
            groupe(a.niveaux[0], dec),
            groupe(a.niveaux[a.niveaux.len() - 1], dec)
        );
    } else if a.croisements.len() > 1 {
        println!(
            "\nAttention : le profil croise zéro {} fois. Le niveau retenu est le plus \
             proche du spot ; le régime n'est pas une simple bascule au-dessus / en dessous.",
            a.croisements.len()
        );
    }

    if let Some(e) = a.ecart_gamma
        && e.relatif.abs() >= 0.05
    {
        let (ech, unite) = echelle(&[e.iv, e.publie]);
        println!(
            "\nAttention : le gamma recalculé depuis l'IV donne {} {unite} $ de GEX,\n\
             et le gamma publié par la source {} — soit {:+.0}% d'écart. Les deux lectures sont\n\
             disponibles via --gamma-source ; l'écart se creuse sur les échéances courtes.",
            groupe(e.iv / ech, 2),
            groupe(e.publie / ech, 2),
            e.relatif * 100.0
        );
    }

    if let (Some(gex), Some(charm)) = (a.part_courtes.gex, a.part_courtes.charm)
        && gex >= 0.15
    {
        println!(
            "\nAttention : les échéances à 0-1 jour portent {:.0}% du GEX, {:.0}% du charm. \
             Leurs greeks sont\ninstables sur des données différées — compare avec --dte-min 2 \
             avant de conclure.",
            gex * 100.0,
            charm * 100.0
        );
    }
}

/// La ligne d'historique d'une analyse.
fn ligne_historique(a: &Analyse, args: &Arguments, dte_max: Option<i64>) -> LigneHistorique {
    LigneHistorique {
        instant: a.releve.0,
        ticker: args.produit.to_uppercase(),
        dte_max,
        source_gamma: match args.gamma_source {
            SourceCli::Iv => "iv",
            SourceCli::Published => "published",
        }
        .to_string(),
        convention: match args.time_convention {
            ConventionCli::Heures => "heures",
            ConventionCli::Bourse => "bourse",
        }
        .to_string(),
        spot: a.spot,
        gex: a.gex,
        zero_gamma: a.zero_gamma,
        call_wall: a.murs.call,
        put_wall: a.murs.put,
        call_wall_oi: a.murs.call_oi,
        put_wall_oi: a.murs.put_oi,
        charm: a.charm,
        vanna: a.vanna,
        strikes: a.par_strike.len(),
        echeances: a
            .lignes
            .iter()
            .map(|l| l.ligne.echeance)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
    }
}

/// Ce qui a bougé depuis le passage précédent.
///
/// Le zero gamma se déplace peu en séance — l'open interest ne bouge qu'une fois
/// par jour. Ce qui bouge, c'est la DISTANCE du prix à ce niveau, et c'est elle
/// qui décide du régime dans lequel on se trouve.
fn ecart_depuis(precedent: &Analyse, actuel: &Analyse, dec: usize) {
    let distance = |a: &Analyse| a.zero_gamma.map(|z| (a.spot - z) / z * 100.0);
    let mut morceaux = vec![format!(
        "spot {}",
        groupe_signe(actuel.spot - precedent.spot, dec)
    )];
    if let (Some(av), Some(ap)) = (precedent.zero_gamma, actuel.zero_gamma) {
        morceaux.push(format!("zero gamma {}", groupe_signe(ap - av, dec)));
    }
    if let (Some(av), Some(ap)) = (distance(precedent), distance(actuel)) {
        morceaux.push(format!(
            "distance au zero gamma {}%",
            groupe_signe(ap - av, 2)
        ));
    }
    println!("  depuis le relevé précédent : {}", morceaux.join(", "));
}

/// Un passage : lire, analyser, afficher.
fn un_passage(
    source: &Path,
    params: &Parametres,
    args: &Arguments,
    dte_max: Option<i64>,
) -> Result<Analyse, String> {
    let chaine = lire_releve(source).map_err(|e| e.to_string())?;
    let analyse = analyser(&chaine, params).map_err(|e| e.to_string())?;
    afficher(&analyse, &args.produit.to_uppercase(), args, dte_max);
    Ok(analyse)
}

/// Échantillonne la séance en boucle, un relevé par passage.
///
/// Un passage n'écrit dans l'historique que si le relevé a réellement avancé : le
/// collecteur réécrit le courant toutes les quinze secondes, donc un --watch plus
/// pressé que lui relit le même fichier, et le réenregistrer n'ajouterait qu'une
/// ligne identique.
fn suivre(
    source: &Path,
    params: &Parametres,
    args: &Arguments,
    dte_max: Option<i64>,
) -> Result<(), String> {
    let intervalle = Duration::from_secs(duree(args.watch.as_deref().unwrap_or("30"))?);
    let total = Duration::from_secs(duree(&args.watch_duration)?);
    let fin = Instant::now() + total;
    println!(
        "Suivi de {} toutes les {}s pendant {:.1}h — Ctrl+C pour arrêter.
",
        args.produit.to_uppercase(),
        intervalle.as_secs(),
        total.as_secs_f64() / 3600.0
    );

    let mut precedent: Option<Analyse> = None;
    let mut vu: Option<InstantReleve> = None;
    let (mut passages, mut enregistres) = (0u32, 0u32);

    loop {
        passages += 1;
        match un_passage(source, params, args, dte_max) {
            Ok(a) => {
                if let Some(avant) = &precedent {
                    ecart_depuis(avant, &a, if a.spot < 10.0 { 4 } else { 2 });
                }
                let avance = vu != Some(a.releve);
                if avance && !args.no_history {
                    enregistrer(&args.history, &ligne_historique(&a, args, dte_max))
                        .map_err(|e| format!("historique : {e}"))?;
                    enregistres += 1;
                } else if !avance {
                    println!("  (relevé inchangé depuis le passage précédent — rien à enregistrer)");
                }
                vu = Some(a.releve);
                precedent = Some(a);
            }
            // Un relevé manqué n'arrête pas le suivi : le collecteur peut être en
            // train de réécrire le fichier, et la séance continue sans nous.
            Err(e) => println!("  relevé manqué ({e}) — on continue"),
        }
        println!();

        let reste = fin.saturating_duration_since(Instant::now());
        if reste.is_zero() {
            break;
        }
        std::thread::sleep(intervalle.min(reste));
    }

    println!(
        "{passages} relevés, {enregistres} enregistrés dans {} (les autres étaient inchangés).",
        args.history.display()
    );
    Ok(())
}

fn executer() -> Result<(), String> {
    let args = Arguments::parse();

    // --watch relit sa source à intervalle régulier. Sur une archive horodatée
    // c'est absurde : elle ne bougera plus. Sur le relevé courant c'est l'usage
    // même, puisque le collecteur le réécrit toutes les quinze secondes.
    if args.watch.is_some() && args.replay.is_some() {
        return Err("--watch et --replay s'excluent : une archive ne bouge plus.                     Pour suivre un relevé vivant, omets --replay."
            .to_string());
    }

    let source = args
        .replay
        .clone()
        .unwrap_or_else(|| chemin_courant(&args.dir, &args.produit));
    if !source.exists() {
        // Un relevé absent n'est pas une panne mais un collecteur qui ne tourne
        // pas : le dire évite une trace de fichier introuvable sans indication.
        return Err(format!(
            "Aucun relevé à lire en {}. Lance le collecteur : gex-collector {}",
            source.display(),
            args.produit
        ));
    }

    let taille = match args.contract_size {
        Some(t) => t,
        // Le multiplicateur ne se devine pas : un produit inconnu est refusé,
        // parce qu'un GEX faux d'un facteur entier reste un nombre plausible.
        None => multiplicateur(&args.produit).map_err(|e| e.to_string())?,
    };
    let dte_max = (!args.toutes_echeances).then_some(args.dte_max);

    let params = Parametres {
        taille_contrat: taille,
        dte_max,
        dte_min: args.dte_min,
        plage: args.range,
        source_gamma: match args.gamma_source {
            SourceCli::Iv => SourceGamma::Iv,
            SourceCli::Published => SourceGamma::Publie,
        },
        convention: match args.time_convention {
            ConventionCli::Heures => Convention::Heures,
            ConventionCli::Bourse => Convention::Bourse,
        },
        regime_vol: match args.vol_regime {
            RegimeCli::StickyStrike => RegimeVol::StickyStrike,
            RegimeCli::StickyMoneyness => RegimeVol::StickyMoneyness,
        },
        ..Default::default()
    };

    if args.watch.is_some() {
        return suivre(&source, &params, &args, dte_max);
    }
    let analyse = un_passage(&source, &params, &args, dte_max)?;
    if !args.no_history {
        enregistrer(&args.history, &ligne_historique(&analyse, &args, dte_max))
            .map_err(|e| format!("historique : {e}"))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match executer() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("Erreur : {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_milliers_sont_groupes() {
        assert_eq!(groupe(29_305.75, 2), "29,305.75");
        assert_eq!(groupe(-357.42, 2), "-357.42");
        assert_eq!(groupe(20.0, 0), "20");
        assert_eq!(groupe(1_234_567.0, 0), "1,234,567");
        assert_eq!(groupe(0.5, 4), "0.5000");
    }

    /// Un GEX de plusieurs milliards ne se lit pas en millions.
    #[test]
    fn l_echelle_suit_l_ordre_de_grandeur() {
        assert_eq!(echelle(&[-2.16e9]).1, "milliards");
        assert_eq!(echelle(&[-3.57e8]).1, "millions");
        assert_eq!(echelle(&[]).1, "millions");
    }

    /// Le sens du flux de couverture est toute l'information : un charm positif
    /// dit que les dealers doivent vendre, un negatif qu'ils doivent racheter.
    #[test]
    fn le_signe_est_toujours_ecrit() {
        assert_eq!(groupe_signe(3.18, 2), "+3.18");
        assert_eq!(groupe_signe(-15.93, 2), "-15.93");
        assert_eq!(groupe_signe(0.0, 2), "+0.00");
    }

    #[test]
    fn les_durees_se_lisent_avec_ou_sans_unite() {
        assert_eq!(duree("6h").unwrap(), 21_600);
        assert_eq!(duree("90m").unwrap(), 5_400);
        assert_eq!(duree("30s").unwrap(), 30);
        assert_eq!(duree("300").unwrap(), 300);
        assert_eq!(duree("1.5h").unwrap(), 5_400);
        assert_eq!(duree(" 5M ").unwrap(), 300);
    }

    /// Une durée illisible doit se dire, pas se transformer en zéro : un
    /// intervalle nul ferait tourner la boucle a plein régime sur le disque.
    #[test]
    fn une_duree_illisible_est_refusee() {
        assert!(duree("bientot").is_err());
        assert!(duree("").is_err());
        assert!(duree("h").is_err());
    }

    #[test]
    fn un_niveau_absent_s_affiche_en_n_a() {
        assert_eq!(optionnel(None, 2), "n/a");
        assert_eq!(optionnel(Some(29_200.0), 2), "29,200.00");
    }
}
