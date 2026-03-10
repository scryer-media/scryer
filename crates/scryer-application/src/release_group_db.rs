use crate::scoring_weights::ScoringWeights;

/// Reputation tier for a release group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupTier {
    /// Top-tier groups (e.g. TRaSH Tier 01 WEB, Tier 01 Remux)
    Gold,
    /// Great groups (e.g. TRaSH Tier 02)
    Silver,
    /// Good groups (e.g. TRaSH Tier 03)
    Bronze,
    /// Known-bad groups (LQ, bad dual audio)
    Banned,
}

/// What source context a group is known for.
/// A group might be Gold for WEB but unknown for BluRay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceContext {
    Web,
    BluRay,
    UhdBluRay,
    Remux,
    Anime,
    /// Applies regardless of source (e.g. banned groups).
    Any,
}

#[derive(Debug, Clone, Copy)]
pub struct GroupEntry {
    pub name: &'static str,
    pub tier: GroupTier,
    pub source_context: SourceContext,
}

/// Look up a release group's tier, considering source context.
///
/// Strategy:
/// 1. Try exact match on (name, source_context) derived from the release
/// 2. Fall back to (name, Any) for groups that are tier-rated regardless of source
/// 3. No match → None (caller applies `group_unknown_penalty`)
pub fn lookup_group(name: &str, source: Option<&str>, is_remux: bool) -> Option<&'static GroupEntry> {
    let name_upper = name.to_ascii_uppercase();
    let ctx = source_to_context(source, is_remux);

    // Try source-specific match first
    if let Some(entry) = GROUPS.iter().find(|g| {
        g.name.eq_ignore_ascii_case(&name_upper) && g.source_context == ctx
    }) {
        return Some(entry);
    }

    // Fall back to Any context (banned groups, etc.)
    GROUPS.iter().find(|g| {
        g.name.eq_ignore_ascii_case(&name_upper) && g.source_context == SourceContext::Any
    })
}

/// Map a parsed source string + remux flag to our SourceContext.
fn source_to_context(source: Option<&str>, is_remux: bool) -> SourceContext {
    if is_remux {
        return SourceContext::Remux;
    }
    match source {
        Some("WEB-DL") | Some("WEBRIP") => SourceContext::Web,
        Some("BLURAY") => SourceContext::BluRay,
        _ => SourceContext::Any,
    }
}

/// Apply release group scoring to a decision.
///
/// Uses the group database to look up the release group's tier for its source
/// context, then applies the corresponding weight from the persona.
pub fn apply_release_group_scoring(
    weights: &ScoringWeights,
    group: Option<&str>,
    source: Option<&str>,
    is_remux: bool,
) -> (& 'static str, i32) {
    let Some(name) = group else {
        return ("group_unknown", weights.group_unknown_penalty);
    };

    if name.is_empty() {
        return ("group_unknown", weights.group_unknown_penalty);
    }

    match lookup_group(name, source, is_remux) {
        Some(entry) => match entry.tier {
            GroupTier::Gold => ("group_gold", weights.group_gold),
            GroupTier::Silver => ("group_silver", weights.group_silver),
            GroupTier::Bronze => ("group_bronze", weights.group_bronze),
            GroupTier::Banned => ("group_banned", weights.group_banned),
        },
        None => ("group_unknown", weights.group_unknown_penalty),
    }
}

// ─── Group database ──────────────────────────────────────────────────────────
//
// Sourced from TRaSH Guides (github.com/TRaSH-Guides/Guides) and community
// sources. Maintained at release time via AI-validated review — see
// scripts/prompts/validate-release-data.md.
//
// Tiers follow TRaSH scoring:
//   Tier 01 → Gold    (best quality for that source context)
//   Tier 02 → Silver  (great quality)
//   Tier 03 → Bronze  (good quality)
//   LQ/Bad  → Banned  (known problematic)

macro_rules! group {
    ($name:expr, $tier:ident, $ctx:ident) => {
        GroupEntry { name: $name, tier: GroupTier::$tier, source_context: SourceContext::$ctx }
    };
}

static GROUPS: &[GroupEntry] = &[
    // ── WEB Tier 01 (Gold) ───────────────────────────────────────────────────
    group!("ABBIE", Gold, Web),
    group!("ABBiE", Gold, Web),
    group!("AJP69", Gold, Web),
    group!("APEX", Gold, Web),
    group!("PAXA", Gold, Web),
    group!("PEXA", Gold, Web),
    group!("XEPA", Gold, Web),
    group!("BLUTONiUM", Gold, Web),
    group!("BYNDR", Gold, Web),
    group!("CasStudio", Gold, Web),
    group!("CMRG", Gold, Web),
    group!("CRFW", Gold, Web),
    group!("CRUD", Gold, Web),
    group!("CtrlHD", Gold, Web),
    group!("FLUX", Gold, Web),
    group!("GNOME", Gold, Web),
    group!("HONE", Gold, Web),
    group!("KiNGS", Gold, Web),
    group!("Kitsune", Gold, Web),
    group!("monkee", Gold, Web),
    group!("NOSiViD", Gold, Web),
    group!("NTb", Gold, Web),
    group!("NTG", Gold, Web),
    group!("QOQ", Gold, Web),
    group!("RAWR", Gold, Web),
    group!("RTN", Gold, Web),
    group!("SiC", Gold, Web),
    group!("T6D", Gold, Web),
    group!("TEPES", Gold, Web),
    group!("TheFarm", Gold, Web),
    group!("TOMMY", Gold, Web),
    group!("ViSUM", Gold, Web),
    group!("ZoroSenpai", Gold, Web),

    // ── WEB Tier 02 (Silver) ─────────────────────────────────────────────────
    group!("3cTWeB", Silver, Web),
    group!("BTW", Silver, Web),
    group!("Chotab", Silver, Web),
    group!("Cinefeel", Silver, Web),
    group!("CiT", Silver, Web),
    group!("Coo7", Silver, Web),
    group!("dB", Silver, Web),
    group!("DEEP", Silver, Web),
    group!("END", Silver, Web),
    group!("ETHiCS", Silver, Web),
    group!("FC", Silver, Web),
    group!("Flights", Silver, Web),
    group!("iJP", Silver, Web),
    group!("iKA", Silver, Web),
    group!("iT00NZ", Silver, Web),
    group!("JETIX", Silver, Web),
    group!("KHN", Silver, Web),
    group!("KiMCHI", Silver, Web),
    group!("LAZY", Silver, Web),
    group!("MiU", Silver, Web),
    group!("MZABI", Silver, Web),
    group!("NPMS", Silver, Web),
    group!("NYH", Silver, Web),
    group!("orbitron", Silver, Web),
    group!("PHOENiX", Silver, Web),
    group!("playWEB", Silver, Web),
    group!("PSiG", Silver, Web),
    group!("ROCCaT", Silver, Web),
    group!("RTFM", Silver, Web),
    group!("SA89", Silver, Web),
    group!("SbR", Silver, Web),
    group!("SDCC", Silver, Web),
    group!("SIGMA", Silver, Web),
    group!("SMURF", Silver, Web),
    group!("SPiRiT", Silver, Web),
    group!("TVSmash", Silver, Web),
    group!("WELP", Silver, Web),
    group!("XEBEC", Silver, Web),
    group!("4KBEC", Silver, Web),
    group!("CEBEX", Silver, Web),

    // ── WEB Tier 03 (Bronze) ─────────────────────────────────────────────────
    group!("BLOOM", Bronze, Web),
    group!("Dooky", Bronze, Web),
    group!("DRACULA", Bronze, Web),
    group!("GNOMiSSiON", Bronze, Web),
    group!("HHWEB", Bronze, Web),
    group!("NINJACENTRAL", Bronze, Web),
    group!("SiGMA", Bronze, Web),
    group!("SLiGNOME", Bronze, Web),
    group!("SwAgLaNdEr", Bronze, Web),
    group!("T4H", Bronze, Web),
    group!("ViSiON", Bronze, Web),

    // ── HD BluRay Tier 01 (Gold) ─────────────────────────────────────────────
    group!("BBQ", Gold, BluRay),
    group!("BMF", Gold, BluRay),
    group!("c0kE", Gold, BluRay),
    group!("Chotab", Gold, BluRay),
    group!("CRiSC", Gold, BluRay),
    group!("CtrlHD", Gold, BluRay),
    group!("D-Z0N3", Gold, BluRay),
    group!("Dariush", Gold, BluRay),
    group!("decibeL", Gold, BluRay),
    group!("DON", Gold, BluRay),
    group!("EbP", Gold, BluRay),
    group!("EDPH", Gold, BluRay),
    group!("Geek", Gold, BluRay),
    group!("LolHD", Gold, BluRay),
    group!("NCmt", Gold, BluRay),
    group!("NTb", Gold, BluRay),
    group!("PTer", Gold, BluRay),
    group!("TayTO", Gold, BluRay),
    group!("TDD", Gold, BluRay),
    group!("TnP", Gold, BluRay),
    group!("VietHD", Gold, BluRay),
    group!("ZQ", Gold, BluRay),
    group!("ZoroSenpai", Gold, BluRay),

    // ── HD BluRay Tier 02 (Silver) ───────────────────────────────────────────
    group!("ATELiER", Silver, BluRay),
    group!("EA", Silver, BluRay),
    group!("HiDt", Silver, BluRay),
    group!("HiSD", Silver, BluRay),
    group!("iFT", Silver, BluRay),
    group!("QOQ", Silver, BluRay),
    group!("SA89", Silver, BluRay),
    group!("sbR", Silver, BluRay),

    // ── HD BluRay Tier 03 (Bronze) ───────────────────────────────────────────
    group!("BHDStudio", Bronze, BluRay),
    group!("hallowed", Bronze, BluRay),
    group!("HiFi", Bronze, BluRay),
    group!("HONE", Bronze, BluRay),
    group!("LoRD", Bronze, BluRay),
    group!("playHD", Bronze, BluRay),
    group!("SPHD", Bronze, BluRay),
    group!("W4NK3R", Bronze, BluRay),

    // ── UHD BluRay Tier 01 (Gold) ────────────────────────────────────────────
    group!("CtrlHD", Gold, UhdBluRay),
    group!("MainFrame", Gold, UhdBluRay),
    group!("DON", Gold, UhdBluRay),
    group!("W4NK3R", Gold, UhdBluRay),

    // ── UHD BluRay Tier 02 (Silver) ──────────────────────────────────────────
    group!("HQMUX", Silver, UhdBluRay),

    // ── UHD BluRay Tier 03 (Bronze) ──────────────────────────────────────────
    group!("BHDStudio", Bronze, UhdBluRay),
    group!("hallowed", Bronze, UhdBluRay),
    group!("HONE", Bronze, UhdBluRay),
    group!("PTer", Bronze, UhdBluRay),
    group!("SPHD", Bronze, UhdBluRay),
    group!("WEBDV", Bronze, UhdBluRay),

    // ── Remux Tier 01 (Gold) ─────────────────────────────────────────────────
    group!("3L", Gold, Remux),
    group!("BiZKiT", Gold, Remux),
    group!("BLURANiUM", Gold, Remux),
    group!("BMF", Gold, Remux),
    group!("CiNEPHiLES", Gold, Remux),
    group!("FraMeSToR", Gold, Remux),
    group!("PiRAMiDHEAD", Gold, Remux),
    group!("PmP", Gold, Remux),
    group!("WiLDCAT", Gold, Remux),
    group!("ZQ", Gold, Remux),

    // ── Remux Tier 02 (Silver) ───────────────────────────────────────────────
    group!("12GaugeShotgun", Silver, Remux),
    group!("ATELiER", Silver, Remux),
    group!("decibeL", Silver, Remux),
    group!("EPSiLON", Silver, Remux),
    group!("HiFi", Silver, Remux),
    group!("KRaLiMaRKo", Silver, Remux),
    group!("NCmt", Silver, Remux),
    group!("playBD", Silver, Remux),
    group!("PTer", Silver, Remux),
    group!("SiCFoI", Silver, Remux),
    group!("SURFINBIRD", Silver, Remux),
    group!("TEPES", Silver, Remux),
    group!("TRiToN", Silver, Remux),

    // ── Remux Tier 03 (Bronze) ───────────────────────────────────────────────
    group!("iFT", Bronze, Remux),
    group!("NTb", Bronze, Remux),
    group!("PTP", Bronze, Remux),
    group!("SumVision", Bronze, Remux),
    group!("TOA", Bronze, Remux),

    // ── Anime Tier 01 (Gold) — BD ────────────────────────────────────────────
    group!("DemiHuman", Gold, Anime),
    group!("FLE", Gold, Anime),
    group!("Flugel", Gold, Anime),
    group!("LYS1TH3A", Gold, Anime),
    group!("Moxie", Gold, Anime),
    group!("NAN0", Gold, Anime),
    group!("sam", Gold, Anime),
    group!("smol", Gold, Anime),
    group!("SoM", Gold, Anime),
    group!("ZR", Gold, Anime),

    // ── Anime Tier 02 (Silver) — BD ──────────────────────────────────────────
    group!("Aergia", Silver, Anime),
    group!("Arg0", Silver, Anime),
    group!("Arid", Silver, Anime),
    group!("FateSucks", Silver, Anime),
    group!("hydes", Silver, Anime),
    group!("hchcsen", Silver, Anime),
    group!("JOHNTiTOR", Silver, Anime),
    group!("JySzE", Silver, Anime),
    group!("koala", Silver, Anime),
    group!("Kulot", Silver, Anime),
    group!("LostYears", Silver, Anime),
    group!("Lulu", Silver, Anime),
    group!("Meakes", Silver, Anime),
    group!("Orphan", Silver, Anime),
    group!("PMR", Silver, Anime),
    group!("Vodes", Silver, Anime),
    group!("WAP", Silver, Anime),
    group!("YURI", Silver, Anime),
    group!("ZeroBuild", Silver, Anime),

    // ── Anime Tier 03 (Bronze) — BD ──────────────────────────────────────────
    group!("ARC", Bronze, Anime),
    group!("BBT-RMX", Bronze, Anime),
    group!("cappybara", Bronze, Anime),
    group!("ChucksMux", Bronze, Anime),
    group!("CRUCiBLE", Bronze, Anime),
    group!("Doc", Bronze, Anime),
    group!("fig", Bronze, Anime),
    group!("Headpatter", Bronze, Anime),
    group!("Inka-Subs", Bronze, Anime),
    group!("LaCroiX", Bronze, Anime),
    group!("Legion", Bronze, Anime),
    group!("Mehul", Bronze, Anime),
    group!("MTBB", Bronze, Anime),
    group!("Mysteria", Bronze, Anime),
    group!("Netaro", Bronze, Anime),
    group!("Noiy", Bronze, Anime),
    group!("npz", Bronze, Anime),
    group!("NTRX", Bronze, Anime),
    group!("Okay-Subs", Bronze, Anime),
    group!("P9", Bronze, Anime),
    group!("RUDY", Bronze, Anime),
    group!("RaiN", Bronze, Anime),
    group!("RMX", Bronze, Anime),
    group!("Sekkon", Bronze, Anime),
    group!("Serendipity", Bronze, Anime),
    group!("sgt", Bronze, Anime),
    group!("SubsMix", Bronze, Anime),
    group!("uba", Bronze, Anime),

    // ── Banned: LQ (Low Quality) groups ──────────────────────────────────────
    group!("24xHD", Banned, Any),
    group!("4K4U", Banned, Any),
    group!("AOC", Banned, Any),
    group!("AROMA", Banned, Any),
    group!("aXXo", Banned, Any),
    group!("BARC0DE", Banned, Any),
    group!("beAst", Banned, Any),
    group!("C1NEM4", Banned, Any),
    group!("C4K", Banned, Any),
    group!("CHD", Banned, Any),
    group!("CiNE", Banned, Any),
    group!("CREATiVE24", Banned, Any),
    group!("CrEwSaDe", Banned, Any),
    group!("CTFOH", Banned, Any),
    group!("d3g", Banned, Any),
    group!("DDR", Banned, Any),
    group!("DNL", Banned, Any),
    group!("EuReKA", Banned, Any),
    group!("FaNGDiNG0", Banned, Any),
    group!("FGT", Banned, Any),
    group!("FRDS", Banned, Any),
    group!("GalaxyRG", Banned, Any),
    group!("GPTHD", Banned, Any),
    group!("HDT", Banned, Any),
    group!("HDTime", Banned, Any),
    group!("HDWinG", Banned, Any),
    group!("iNTENSO", Banned, Any),
    group!("iPlanet", Banned, Any),
    group!("KIRA", Banned, Any),
    group!("LAMA", Banned, Any),
    group!("Leffe", Banned, Any),
    group!("LiGaS", Banned, Any),
    group!("LUCY", Banned, Any),
    group!("MeGusta", Banned, Any),
    group!("mHD", Banned, Any),
    group!("mSD", Banned, Any),
    group!("MySiLU", Banned, Any),
    group!("nHD", Banned, Any),
    group!("nikt0", Banned, Any),
    group!("nSD", Banned, Any),
    group!("OFT", Banned, Any),
    group!("Pahe", Banned, Any),
    group!("PATOMiEL", Banned, Any),
    group!("PRODJi", Banned, Any),
    group!("PSA", Banned, Any),
    group!("PTNK", Banned, Any),
    group!("RARBG", Banned, Any),
    group!("RDN", Banned, Any),
    group!("SANTi", Banned, Any),
    group!("SHD", Banned, Any),
    group!("ShieldBearer", Banned, Any),
    group!("STUTTERSHIT", Banned, Any),
    group!("SUNSCREEN", Banned, Any),
    group!("TBS", Banned, Any),
    group!("TEKNO3D", Banned, Any),
    group!("Tigole", Banned, Any),
    group!("TIKO", Banned, Any),
    group!("WAF", Banned, Any),
    group!("WiKi", Banned, Any),
    group!("x0r", Banned, Any),
    group!("YIFY", Banned, Any),
    group!("YTS", Banned, Any),
    group!("Zeus", Banned, Any),
    group!("EVO", Banned, Any),
    group!("D3US", Banned, Any),
    group!("PiRaTeS", Banned, Any),

    // ── Banned: Bad dual audio groups ─────────────────────────────────────────
    group!("alfaHD", Banned, Any),
    group!("BAT", Banned, Any),
    group!("BlackBit", Banned, Any),
    group!("BNd", Banned, Any),
    group!("EXTREME", Banned, Any),
    group!("FF", Banned, Any),
    group!("FOXX", Banned, Any),
    group!("G4RiS", Banned, Any),
    group!("GUEIRA", Banned, Any),
    group!("LCD", Banned, Any),
    group!("MGE", Banned, Any),
    group!("N3G4N", Banned, Any),
    group!("ONLYMOViE", Banned, Any),
    group!("PD", Banned, Any),
    group!("PTHome", Banned, Any),
    group!("RiPER", Banned, Any),
    group!("RK", Banned, Any),
    group!("SiGLA", Banned, Any),
    group!("Tars", Banned, Any),
    group!("TM", Banned, Any),
    group!("tokar86a", Banned, Any),
    group!("TURG", Banned, Any),
    group!("TvR", Banned, Any),
    group!("vnlls", Banned, Any),
    group!("WTV", Banned, Any),
    group!("Yatogam1", Banned, Any),
    group!("YusukeFLA", Banned, Any),
    group!("ZigZag", Banned, Any),
    group!("ZNM", Banned, Any),
    group!("BiOMA", Banned, Any),
    group!("Cory", Banned, Any),

    // ── Banned: Anime LQ groups ──────────────────────────────────────────────
    group!("AnimeRG", Banned, Anime),
    group!("BakedFish", Banned, Anime),
    group!("DeadFish", Banned, Anime),
    group!("SpaceFish", Banned, Anime),
    group!("CBB", Banned, Anime),
    group!("Cleo", Banned, Anime),
    group!("DB", Banned, Anime),
    group!("iPUNISHER", Banned, Anime),
    group!("Judas", Banned, Anime),
    group!("Kanjouteki", Banned, Anime),
    group!("LoliHouse", Banned, Anime),
    group!("MiniFreeza", Banned, Anime),
    group!("MiniTheatre", Banned, Anime),
    group!("NoobSubs", Banned, Anime),
    group!("NemDiggers", Banned, Anime),
    group!("SSA", Banned, Anime),
    group!("youshikibi", Banned, Anime),
    group!("Hakata Ramen", Banned, Anime),
    group!("EMBER", Banned, Anime),
    group!("EDGE", Banned, Anime),
    group!("project-gxs", Banned, Anime),
    group!("Bonkai77", Banned, Anime),
    group!("HorribleSubs", Banned, Anime),
    group!("HorribleRips", Banned, Anime),
    group!("SubsPlease", Banned, Anime),
    group!("Erai-Raws", Banned, Anime),
];

#[cfg(test)]
#[path = "release_group_db_tests.rs"]
mod release_group_db_tests;
