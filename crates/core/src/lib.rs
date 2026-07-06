#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Team {
    BloodEagle,
    DiamondSword,
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