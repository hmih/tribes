#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Team {
    BloodEagle,
    DiamondSword,
}

impl Team {
    /// Returns the team implied by a UE3 class name suffix.
    ///
    /// Tribes gameplay actor classes carry a team suffix in the class name
    /// (e.g. `TrCTFBase_BloodEagle`, `TrInventoryStation_DiamondSword`).
    /// Returns `None` for neutral classes (volumes, world info, etc.).
    pub fn from_class(class: &str) -> Option<Self> {
        if class.ends_with("_BloodEagle") {
            Some(Self::BloodEagle)
        } else if class.ends_with("_DiamondSword") {
            Some(Self::DiamondSword)
        } else {
            None
        }
    }

    /// Decodes a UE3 `TeamIndex`/`TeamNumber` property value.
    ///
    /// In Tribes, `0` is BloodEagle and `1` is DiamondSword. A missing or
    /// unparseable value defaults to BloodEagle (the game's default team).
    pub fn from_index(index: Option<i64>) -> Self {
        match index {
            Some(1) => Self::DiamondSword,
            _ => Self::BloodEagle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ArmorType {
    Light,
    Medium,
    Heavy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum WeaponSlot {
    Primary,
    Secondary,
    Belt,
    Pack,
    Melee,
}
