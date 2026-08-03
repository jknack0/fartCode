const ADJECTIVES: &[&str] = &[
    "afraid", "all", "beige", "better", "big", "blue", "bold", "brave", "breezy", "bright",
    "brown", "bumpy", "busy", "calm", "chatty", "chilly", "chubby", "clean", "clear", "clever",
    "cold", "common", "cool", "cozy", "crisp", "cuddly", "curly", "curvy", "cute", "cyan", "dark",
    "deep", "dirty", "dry", "dull", "eager", "early", "easy", "eight", "eighty", "eleven", "empty",
    "every", "fair", "famous", "fancy", "fast", "few", "fiery", "fifty", "fine", "five", "flat",
    "floppy", "fluffy", "forty", "four", "frank", "free", "fresh", "fruity", "full", "funky",
    "funny", "fuzzy", "gentle", "giant", "gold", "good", "goofy", "great", "green", "grumpy",
    "happy", "heavy", "hip", "honest", "hot", "huge", "humble", "hungry", "icy", "itchy", "jolly",
    "khaki", "kind", "large", "late", "lazy", "legal", "lemon", "light", "little", "long", "loose",
    "loud", "lovely", "lucky", "major", "many", "metal", "mighty", "modern", "moody", "neat",
    "new", "nice", "nine", "ninety", "odd", "old", "olive", "open", "orange", "perky", "petite",
    "pink", "plain", "plenty", "polite", "pretty", "proud", "public", "puny", "purple", "quick",
    "quiet", "rare", "ready", "real", "red", "rich", "ripe", "salty", "seven", "shaggy", "shaky",
    "sharp", "shiny", "short", "shy", "silent", "silly", "silver", "six", "sixty", "slick",
    "slimy", "slow", "small", "smart", "smooth", "social", "soft", "solid", "some", "sour",
    "sparkly", "spicy", "spotty", "stale", "strict", "strong", "sunny", "sweet", "swift", "tall",
    "tame", "tangy", "tasty", "ten", "tender", "thick", "thin", "thirty", "three", "tidy", "tiny",
    "tired", "tough", "tricky", "true", "twelve", "twenty", "two", "upset", "vast", "violet",
    "wacky", "warm", "wet", "whole", "wicked", "wide", "wild", "wise", "witty", "yellow", "young",
    "yummy",
];

const NOUNS: &[&str] = &[
    "actors", "ads", "adults", "aliens", "animals", "ants", "apes", "apples", "areas", "baboons",
    "badgers", "bags", "balloons", "bananas", "banks", "bars", "baths", "bats", "beans", "bears",
    "beds", "beers", "bees", "berries", "bikes", "birds", "boats", "bobcats", "books", "bottles",
    "boxes", "breads", "brooms", "buckets", "bugs", "buses", "bushes", "buttons", "camels",
    "cameras", "candies", "candles", "canyons", "carpets", "carrots", "cars", "cases", "cats",
    "chairs", "chefs", "chicken", "cities", "clocks", "cloths", "clouds", "clowns", "clubs",
    "coats", "cobras", "coins", "colts", "comics", "cooks", "corners", "cougars", "cows", "crabs",
    "crews", "cups", "cycles", "dancers", "days", "deer", "deserts", "dingos", "dodos", "dogs",
    "dolls", "donkeys", "donuts", "doodles", "doors", "dots", "dragons", "drinks", "dryers",
    "ducks", "eagles", "ears", "eels", "eggs", "emus", "ends", "experts", "eyes", "facts",
    "falcons", "fans", "feet", "files", "flies", "flowers", "forks", "foxes", "friends", "frogs",
    "games", "garlics", "geckos", "geese", "ghosts", "gifts", "glasses", "goats", "grapes",
    "groups", "guests", "hairs", "hands", "hats", "heads", "hoops", "hornets", "horses", "hotels",
    "hounds", "houses", "humans", "icons", "ideas", "impalas", "insects", "islands", "items",
    "jars", "jeans", "jobs", "jokes", "keys", "kids", "kings", "kiwis", "knives", "lamps", "lands",
    "laws", "lemons", "lies", "lights", "lilies", "lines", "lions", "lizards", "llamas", "loops",
    "mails", "mammals", "mangos", "maps", "masks", "meals", "melons", "memes", "meteors", "mice",
    "mirrors", "moles", "moments", "monkeys", "months", "moons", "moose", "mugs", "nails",
    "needles", "news", "nights", "numbers", "olives", "onions", "oranges", "otters", "owls",
    "pandas", "pans", "pants", "papayas", "papers", "parents", "parks", "parrots", "parts",
    "paths", "paws", "peaches", "pears", "peas", "pens", "pets", "phones", "pianos", "pigs",
    "pillows", "places", "planes", "planets", "plants", "plums", "poems", "poets", "points",
    "pots", "pugs", "pumas", "queens", "rabbits", "radios", "rats", "ravens", "readers", "regions",
    "results", "rice", "rings", "rivers", "rockets", "rocks", "rooms", "roses", "rules", "sails",
    "schools", "seals", "seas", "sheep", "shirts", "shoes", "showers", "shrimps", "sides", "signs",
    "singers", "sites", "sloths", "snails", "snakes", "socks", "spiders", "spies", "spoons",
    "squids", "stamps", "stars", "states", "steaks", "streets", "suits", "suns", "swans",
    "symbols", "tables", "taxes", "taxis", "teams", "teeth", "terms", "things", "ties", "tigers",
    "times", "tips", "tires", "toes", "tools", "towns", "toys", "trains", "trams", "trees",
    "turkeys", "turtles", "vans", "views", "walls", "wasps", "waves", "ways", "webs", "weeks",
    "windows", "wings", "wolves", "wombats", "words", "worlds", "worms", "yaks", "years", "zebras",
    "zoos",
];

const VERBS: &[&str] = &[
    "accept", "act", "add", "admire", "agree", "allow", "appear", "argue", "arrive", "ask",
    "attack", "attend", "bake", "bathe", "battle", "beam", "beg", "begin", "behave", "bet", "boil",
    "bow", "brake", "brush", "build", "burn", "buy", "call", "camp", "care", "carry", "change",
    "cheat", "check", "cheer", "chew", "clap", "clean", "cough", "count", "cover", "crash",
    "create", "cross", "cry", "cut", "dance", "decide", "deny", "design", "dig", "divide", "do",
    "double", "doubt", "draw", "dream", "dress", "drive", "drop", "drum", "eat", "end", "enjoy",
    "enter", "exist", "fail", "fall", "feel", "fetch", "film", "find", "fix", "flash", "float",
    "flow", "fly", "fold", "follow", "fry", "give", "glow", "go", "grab", "greet", "grin", "grow",
    "guess", "hammer", "hang", "happen", "heal", "hear", "help", "hide", "hope", "hug", "hunt",
    "invent", "invite", "itch", "jam", "jog", "join", "joke", "judge", "juggle", "jump", "kick",
    "kiss", "kneel", "knock", "know", "laugh", "lay", "lead", "learn", "leave", "lick", "lie",
    "like", "listen", "live", "look", "lose", "love", "make", "march", "marry", "mate", "matter",
    "melt", "mix", "move", "nail", "notice", "obey", "occur", "open", "own", "pay", "peel", "pick",
    "play", "poke", "post", "press", "prove", "pull", "pump", "punch", "push", "raise", "read",
    "refuse", "relate", "relax", "remain", "repair", "repeat", "reply", "report", "rescue", "rest",
    "retire", "return", "rhyme", "ring", "roll", "rule", "run", "rush", "say", "scream", "search",
    "see", "sell", "send", "serve", "shake", "share", "shave", "shine", "shop", "shout", "show",
    "sin", "sing", "sink", "sip", "sit", "sleep", "slide", "smash", "smell", "smile", "smoke",
    "sneeze", "sniff", "sort", "speak", "spend", "stand", "stare", "start", "stay", "stick",
    "stop", "strive", "study", "swim", "switch", "take", "talk", "tan", "tap", "taste", "teach",
    "tease", "tell", "thank", "think", "throw", "tickle", "tie", "trade", "train", "travel", "try",
    "turn", "type", "unite", "vanish", "visit", "wait", "walk", "warn", "wash", "watch", "wave",
    "wear", "win", "wink", "wish", "wonder", "work", "worry", "write", "yawn", "yell",
];

use crate::tasks::model::LinkedIssue;

/// Maximum generated task name / branch segment length (reference
/// `MAX_TASK_NAME_LENGTH`).
pub const MAX_TASK_NAME_LENGTH: usize = 64;

/// Sanitizes to `[a-z0-9-]`: lowercase, non-alphanumerics -> `-` (runs
/// collapsed), leading/trailing `-` trimmed, capped at 64 chars (reference
/// `sanitize()` in generateTaskName.ts).
pub fn sanitize_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = false;
    for c in input.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out.truncate(MAX_TASK_NAME_LENGTH);
    out
}

/// Picks a uniformly random word from a list using uuid v4 entropy (no rand
/// dependency; modulo bias is negligible at 2^-122).
fn pick_word<'a>(list: &'a [&'a str]) -> &'a str {
    let u = uuid::Uuid::new_v4().as_u128();
    list[(u % list.len() as u128) as usize]
}
/// Human-id style random name: `adjective-noun-verb` lowercase, hyphenated
/// (reference `humanId({ separator: '-', capitalize: false })` — the default
/// `adjectiveCount: 1, addAdverb: false`). Word lists vendored from
/// `human-id@4.2.0`.
pub fn generate_random_name() -> String {
    format!(
        "{}-{}-{}",
        pick_word(ADJECTIVES),
        pick_word(NOUNS),
        pick_word(VERBS)
    )
}

/// Title-slug name (reference `generateFromInput` via `nbranch`
/// `generateBranchName` with `addRandomSuffix: false`): lowercase, non-alnum
/// -> `-`, collapsed/trimmed, capped at 64. A description is appended to the
/// title (separated by newlines, which become dashes).
pub fn generate_name_from_title(title: &str, description: Option<&str>) -> String {
    let input = match description {
        Some(d) if !d.trim().is_empty() => format!("{title}\n\n{d}"),
        _ => title.to_string(),
    };
    sanitize_name(&input)
}

/// Task-name entry point: explicit title -> slug; no title -> random
/// human-id name when `auto_generate` is true, else `""` (the caller must
/// supply a title when auto-generation is off).
pub fn generate_task_name(
    title: Option<&str>,
    description: Option<&str>,
    auto_generate: bool,
) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(title) => generate_name_from_title(title, description),
        None if auto_generate => generate_random_name(),
        None => String::new(),
    }
}

/// Random 5-char base36 (`[0-9a-z]`) suffix (reference
/// `Math.random().toString(36).slice(2, 7)`).
pub fn random_suffix() -> String {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut value = uuid::Uuid::new_v4().as_u128();
    let mut out = String::with_capacity(5);
    for _ in 0..5 {
        out.push(CHARS[(value % 36) as usize] as char);
        value /= 36;
    }
    out
}

/// Normalizes a branch prefix: trim + strip leading/trailing `/` (internal
/// slashes preserved — reference `normalizeBranchPrefix`).
pub fn normalize_branch_prefix(prefix: &str) -> String {
    prefix.trim().trim_matches('/').to_string()
}

/// Options for `resolve_task_branch_name` (reference `resolveTaskBranchName`).
pub struct BranchNameOptions<'a> {
    pub raw_branch: &'a str,
    pub branch_prefix: Option<&'a str>,
    /// Pre-generated suffix (5 chars base36). Appended only when
    /// `append_random_suffix` is true.
    pub suffix: &'a str,
    pub append_random_suffix: bool,
    pub linked_issue: Option<&'a LinkedIssue>,
    pub disable_random_suffix: bool,
}

impl<'a> BranchNameOptions<'a> {
    pub fn new(raw_branch: &'a str) -> Self {
        Self {
            raw_branch,
            branch_prefix: None,
            suffix: "",
            append_random_suffix: true,
            linked_issue: None,
            disable_random_suffix: false,
        }
    }
}

/// Resolves the task branch name:
/// - **Linear verbatim**: a linked Linear issue with a `branchName` is used
///   unchanged (no prefix, no suffix).
/// - Linear without `branchName` suppresses the suffix (still prefixed).
/// - Otherwise `rawBranch-suffix` when `append_random_suffix` (and not
///   disabled), then `prefix/branch` when a prefix is set.
///
/// The reference does **not** sanitize here — `raw_branch` is expected to
/// come from the (sanitized) task name; Linear verbatim must pass through
/// untouched.
pub fn resolve_task_branch_name(opts: &BranchNameOptions<'_>) -> String {
    if let Some(issue) = opts.linked_issue {
        if issue.provider == "linear" {
            if let Some(branch) = issue
                .branch_name
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
            {
                return branch.to_string();
            }
        }
    }
    let provider_is_linear = opts
        .linked_issue
        .as_ref()
        .is_some_and(|i| i.provider == "linear");
    let append = opts.append_random_suffix && !opts.disable_random_suffix && !provider_is_linear;
    let branch = if append {
        format!("{}-{}", opts.raw_branch, opts.suffix)
    } else {
        opts.raw_branch.to_string()
    };
    match opts.branch_prefix.map(str::trim).filter(|p| !p.is_empty()) {
        Some(prefix) => format!("{}/{}", normalize_branch_prefix(prefix), branch),
        None => branch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::model::LinkedIssue;

    #[test]
    fn sanitize_collapses_and_trims() {
        assert_eq!(sanitize_name("Fix #891 auth!"), "fix-891-auth");
        assert_eq!(sanitize_name("A  B"), "a-b");
        assert_eq!(sanitize_name("---x---"), "x");
        assert_eq!(sanitize_name("CAPS and spaces"), "caps-and-spaces");
        let long = "x".repeat(100);
        assert_eq!(sanitize_name(&long).len(), MAX_TASK_NAME_LENGTH);
    }

    #[test]
    fn random_name_is_human_id_style() {
        let mut names = std::collections::BTreeSet::new();
        for _ in 0..5 {
            let name = generate_random_name();
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "name must be [a-z0-9-]: {name}"
            );
            assert!(!name.contains("--"), "no collapsed dashes: {name}");
            assert!(!name.contains('/'), "no slashes: {name}");
            assert!(name.len() <= MAX_TASK_NAME_LENGTH);
            assert_eq!(name.split('-').count(), 3, "adjective-noun-verb: {name}");
            names.insert(name);
        }
        assert!(names.len() >= 2, "random names must vary");
    }

    #[test]
    fn title_slugs_correctly() {
        assert_eq!(
            generate_name_from_title("Fix auth bug", None),
            "fix-auth-bug"
        );
        assert_eq!(generate_name_from_title("Fix #891", None), "fix-891");
        assert_eq!(
            generate_name_from_title("Refactor", Some("Tighten the loop")),
            "refactor-tighten-the-loop"
        );
    }

    #[test]
    fn task_name_entry_point() {
        assert_eq!(
            generate_task_name(Some("Fix auth"), None, false),
            "fix-auth"
        );
        let random = generate_task_name(None, None, true);
        assert_eq!(random.split('-').count(), 3);
        assert_eq!(generate_task_name(None, None, false), "");
        assert_eq!(generate_task_name(Some("  "), None, false), "");
    }

    #[test]
    fn branch_name_prefix_and_suffix() {
        let opts = BranchNameOptions {
            raw_branch: "fix-auth-bug",
            branch_prefix: Some("ade"),
            suffix: "abc12",
            append_random_suffix: true,
            linked_issue: None,
            disable_random_suffix: false,
        };
        assert_eq!(resolve_task_branch_name(&opts), "ade/fix-auth-bug-abc12");

        let no_suffix = BranchNameOptions {
            append_random_suffix: false,
            ..opts
        };
        assert_eq!(resolve_task_branch_name(&no_suffix), "ade/fix-auth-bug");

        let no_prefix = BranchNameOptions {
            branch_prefix: None,
            ..opts
        };
        assert_eq!(resolve_task_branch_name(&no_prefix), "fix-auth-bug-abc12");

        let disabled = BranchNameOptions {
            disable_random_suffix: true,
            ..opts
        };
        assert_eq!(resolve_task_branch_name(&disabled), "ade/fix-auth-bug");
    }

    #[test]
    fn linear_issue_branch_is_verbatim() {
        let linear = LinkedIssue {
            provider: "linear".into(),
            url: "https://linear.app/x/issue/ABC-1".into(),
            title: "Linear thing".into(),
            identifier: "ABC-1".into(),
            display_identifier: None,
            description: None,
            branch_name: Some("my-linear-branch".into()),
            status: None,
        };
        let opts = BranchNameOptions {
            raw_branch: "fix-auth-bug",
            branch_prefix: Some("ade"),
            suffix: "abc12",
            append_random_suffix: true,
            linked_issue: Some(&linear),
            disable_random_suffix: false,
        };
        assert_eq!(resolve_task_branch_name(&opts), "my-linear-branch");

        // Linear without branchName: suffix suppressed, prefix still applies.
        let linear_no_branch = LinkedIssue {
            branch_name: None,
            ..linear.clone()
        };
        let opts = BranchNameOptions {
            linked_issue: Some(&linear_no_branch),
            ..opts
        };
        assert_eq!(resolve_task_branch_name(&opts), "ade/fix-auth-bug");
    }

    #[test]
    fn prefix_normalized() {
        assert_eq!(normalize_branch_prefix("  ade  "), "ade");
        assert_eq!(normalize_branch_prefix("/team/ade/"), "team/ade");
        assert_eq!(normalize_branch_prefix("/ade"), "ade");
    }

    #[test]
    fn suffix_is_five_base36_chars() {
        for _ in 0..5 {
            let suffix = random_suffix();
            assert_eq!(suffix.len(), 5);
            assert!(
                suffix.chars().all(|c| c.is_ascii_alphanumeric()),
                "suffix must be [0-9a-z]: {suffix}"
            );
        }
    }
}
