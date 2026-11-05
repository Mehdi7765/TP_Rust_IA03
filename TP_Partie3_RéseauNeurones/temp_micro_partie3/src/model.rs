// Le réseau de neurones (Burn).
//
// Documentation des couches : https://docs.rs/burn/latest/burn/nn/index.html
// (Conv1d, MaxPool1d, Linear, LayerNorm, Relu et leurs *Config)

use burn::{
    config::Config,
    module::Module,
    nn,
    nn::modules,
    tensor::{Tensor, backend::Backend},
};

/// Backend utilisé sur la carte : NdArray (CPU, sans std) + Autodiff (calcul des gradients)
pub type ModelBackend = burn::backend::Autodiff<burn::backend::NdArray>;

// TODO 1 : compléter la structure du réseau.
//
// Le réseau enchaîne, dans cet ordre :
//   norm   : LayerNorm       (déjà fait, exemple)
//   conv1  : Conv1d          -> type `modules::conv::Conv1d<B>`
//   act1   : ReLU            -> type `nn::Relu` (pas de paramètre, donc pas de <B>)
//   pool1  : MaxPool1d       -> type `modules::pool::MaxPool1d`
//   conv2  : Conv1d
//   act2   : ReLU
//   pool2  : MaxPool1d
//   linear : Linear          -> type `modules::Linear<B>`
//
// `#[derive(Module)]` génère tout ce qu'il faut pour que Burn retrouve les
// paramètres (poids) de chaque couche, pour l'entraînement.
#[derive(Module, Debug)]
pub struct Model<B: Backend> {
    norm: modules::LayerNorm<B>,
    // Étape 1 : champs du réseau, dans l'ordre de l'enchaînement
    conv1: modules::conv::Conv1d<B>,
    act1: nn::Relu, // pas de paramètre -> pas de <B>
    pool1: modules::pool::MaxPool1d,
    conv2: modules::conv::Conv1d<B>,
    act2: nn::Relu,
    pool2: modules::pool::MaxPool1d,
    linear: modules::Linear<B>,
}

impl<B: Backend> Model<B> {
    /// Entrée : [input_size] échantillons. Sortie : [output_size] scores (logits).
    pub fn forward(&self, x: Tensor<B, 1>) -> Tensor<B, 1> {
        // Les couches 1D de Burn attendent un tenseur [batch, canaux, temps]
        let x = x.unsqueeze::<3>(); // [2000] -> [1, 1, 2000]
        let x = self.norm.forward(x); // normalise l'amplitude du mot

        // TODO 2 : appliquer les couches suivantes, dans l'ordre, avec leur `forward`.
        //
        //  Formes attendues (avec 4 puis 8 filtres) :
        //    conv1 -> [1, 4, 1998]   pool1 -> [1, 4, 499]
        //    conv2 -> [1, 8, 497]    pool2 -> [1, 8, 124]
        //  Puis, avant la couche linéaire :
        //    - aplatir en GARDANT la dimension batch : `x.flatten::<2>(1, 2)` -> [1, 992]
        //      (Linear attend [batch, entrées])
        //    - `self.linear.forward(x)` donne [1, 3]
        //    - revenir à 1 dimension : `.reshape([-1])` -> [3]
        //
        // Astuce : si une forme est fausse, le programme s'arrête avec un panic
        // qui affiche les formes incompatibles. La forme d'un tenseur s'affiche avec
        // `defmt::println!("{}", defmt::Debug2Format(&x.shape()))`.

        // Étape 2 : application des couches dans l'ordre
        let x = self.conv1.forward(x); // [1, 4, 1998]
        let x = self.act1.forward(x);
        let x = self.pool1.forward(x); // [1, 4, 499]
        let x = self.conv2.forward(x); // [1, 8, 497]
        let x = self.act2.forward(x);
        let x = self.pool2.forward(x); // [1, 8, 124]
        // Étape 2 : aplatir en gardant la dimension batch (Linear attend [batch, entrées])
        let x = x.flatten::<2>(1, 2); // [1, 992]
        let x = self.linear.forward(x); // [1, 3]
        // Étape 2 : revenir à 1 dimension
        x.reshape([-1]) // [3]
    }
}

/// Hyperparamètres du réseau. `#[derive(Config)]` génère `ModelConfig::new(input_size, output_size)`
/// et une méthode `with_<champ>(valeur)` pour chaque champ qui a une valeur par défaut.
#[derive(Config, Debug)]
pub struct ModelConfig {
    input_size: usize,
    output_size: usize,
    #[config(default = 3)]
    conv1_kernel_size: usize,
    #[config(default = 2)]
    conv1_channels_count: usize,
    #[config(default = 4)]
    pool1_kernel_size: usize,
    #[config(default = 3)]
    conv2_kernel_size: usize,
    #[config(default = 3)]
    conv2_channels_count: usize,
    #[config(default = 4)]
    pool2_kernel_size: usize,
}

impl ModelConfig {
    pub fn init<B: Backend>(&self, device: &B::Device) -> Model<B> {
        // TODO 3 : calculer la taille d'entrée de la couche linéaire, à partir des champs
        // de la configuration (ne pas écrire 992 en dur : il faut que ça reste juste si
        // on change un hyperparamètre).
        //
        //  - une convolution de noyau k, sans padding, enlève k - 1 échantillons :
        //      L -> L - (k - 1)
        //  - un max-pooling de taille p divise la longueur (division entière) :
        //      L -> L / p
        //  - la couche linéaire reçoit (longueur finale) x (nombre de filtres de conv2)
        //  Vérification : avec 2000 échantillons et 4/8 filtres, on doit trouver 992.

        // TODO 4 : créer chaque couche avec sa configuration, puis `init(device)`.
        //
        //  - `modules::conv::Conv1dConfig::new(canaux_entree, canaux_sortie, noyau)`
        //    (conv1 : 1 canal en entrée, le microphone)
        //  - `nn::Relu::new()`
        //  - `modules::pool::MaxPool1dConfig::new(taille)` : son `init()` ne prend pas
        //    de device (pas de paramètre à stocker)
        //  - `modules::LinearConfig::new(entrees, sorties)`

        // Étape 3 : taille d'entrée de Linear calculée à partir des champs
        // (2000 -> conv1 1998 -> pool1 499 -> conv2 497 -> pool2 124 ; 124 x 8 = 992)
        let len = self.input_size - (self.conv1_kernel_size - 1); // conv1
        let len = len / self.pool1_kernel_size; // pool1
        let len = len - (self.conv2_kernel_size - 1); // conv2
        let len = len / self.pool2_kernel_size; // pool2
        let linear_input_size = len * self.conv2_channels_count;

        // Étape 3 : création de chaque couche avec sa configuration
        Model {
            norm: modules::LayerNormConfig::new(self.input_size).init(device),
            // 1 canal en entrée : le microphone
            conv1: modules::conv::Conv1dConfig::new(1, self.conv1_channels_count, self.conv1_kernel_size)
                .init(device),
            act1: nn::Relu::new(),
            pool1: modules::pool::MaxPool1dConfig::new(self.pool1_kernel_size).init(),
            conv2: modules::conv::Conv1dConfig::new(
                self.conv1_channels_count,
                self.conv2_channels_count,
                self.conv2_kernel_size,
            )
            .init(device),
            act2: nn::Relu::new(),
            pool2: modules::pool::MaxPool1dConfig::new(self.pool2_kernel_size).init(),
            linear: modules::LinearConfig::new(linear_input_size, self.output_size).init(device),
        }
    }
}
