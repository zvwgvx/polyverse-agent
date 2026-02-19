use serde::{Deserialize, Serialize};

/// The mood of the agent, derived from sentiment analysis of user interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mood {
    Excited,
    Happy,
    Neutral,
    Annoyed,
    Angry,
    Sad,
}

impl Default for Mood {
    fn default() -> Self {
        Mood::Neutral
    }
}

impl std::fmt::Display for Mood {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mood::Excited => write!(f, "Excited"),
            Mood::Happy => write!(f, "Happy"),
            Mood::Neutral => write!(f, "Neutral"),
            Mood::Annoyed => write!(f, "Annoyed"),
            Mood::Angry => write!(f, "Angry"),
            Mood::Sad => write!(f, "Sad"),
        }
    }
}

/// The digital biology state of the agent.
/// This is shared across workers and represents the agent's "physical" condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiologyState {
    /// Energy level (0.0 = exhausted, 100.0 = fully charged).
    /// Complex queries drain energy faster. Recovery happens during idle.
    pub energy: f32,

    /// Current mood, influenced by user sentiment analysis.
    pub mood: Mood,

    /// Whether the agent is in sleep/consolidation mode.
    pub is_sleeping: bool,

    /// Mood intensity / valence (-1.0 to 1.0).
    /// Negative = bad mood intensity, Positive = good mood intensity.
    pub mood_valence: f32,
}

impl Default for BiologyState {
    fn default() -> Self {
        Self {
            energy: 100.0,
            mood: Mood::Neutral,
            is_sleeping: false,
            mood_valence: 0.0,
        }
    }
}

impl BiologyState {
    /// Create a new BiologyState with full energy and neutral mood.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain energy by a given amount. Clamps to 0.0 minimum.
    pub fn drain_energy(&mut self, amount: f32) {
        self.energy = (self.energy - amount).max(0.0);
    }

    /// Recover energy by a given amount. Clamps to 100.0 maximum.
    pub fn recover_energy(&mut self, amount: f32) {
        self.energy = (self.energy + amount).min(100.0);
    }

    /// Whether the agent is low on energy (below threshold).
    pub fn is_exhausted(&self) -> bool {
        self.energy < 10.0
    }

    /// Whether the agent is in a negative mood state.
    pub fn is_negative_mood(&self) -> bool {
        matches!(self.mood, Mood::Annoyed | Mood::Angry | Mood::Sad)
    }

    /// Update mood with a new value and valence.
    pub fn update_mood(&mut self, mood: Mood, valence: f32) {
        self.mood = mood;
        self.mood_valence = valence.clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_biology() {
        let bio = BiologyState::new();
        assert_eq!(bio.energy, 100.0);
        assert_eq!(bio.mood, Mood::Neutral);
        assert!(!bio.is_sleeping);
        assert!(!bio.is_exhausted());
    }

    #[test]
    fn test_energy_drain_and_recovery() {
        let mut bio = BiologyState::new();
        bio.drain_energy(30.0);
        assert_eq!(bio.energy, 70.0);

        bio.drain_energy(80.0);
        assert_eq!(bio.energy, 0.0); // clamped

        bio.recover_energy(50.0);
        assert_eq!(bio.energy, 50.0);

        bio.recover_energy(200.0);
        assert_eq!(bio.energy, 100.0); // clamped
    }

    #[test]
    fn test_exhaustion() {
        let mut bio = BiologyState::new();
        assert!(!bio.is_exhausted());
        bio.drain_energy(95.0);
        assert!(bio.is_exhausted());
    }

    #[test]
    fn test_mood_update() {
        let mut bio = BiologyState::new();
        bio.update_mood(Mood::Angry, -0.8);
        assert_eq!(bio.mood, Mood::Angry);
        assert!(bio.is_negative_mood());
        assert_eq!(bio.mood_valence, -0.8);

        // Test valence clamping
        bio.update_mood(Mood::Excited, 5.0);
        assert_eq!(bio.mood_valence, 1.0);
    }
}
