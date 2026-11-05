// Le réseau de neurones (Burn).

use burn::{
    config::Config,
    module::Module,
    nn,
    nn::modules,
    tensor::{Tensor, backend::Backend},
};

/// Backend utilisé sur la carte : NdArray (CPU, sans std) + Autodiff (calcul des gradients)
pub type ModelBackend = burn::backend::Autodiff<burn::backend::NdArray>;

/// Réseau convolutif 1D :
/// LayerNorm -> Conv1d -> ReLU -> MaxPool1d -> Conv1d -> ReLU -> MaxPool1d -> Linear
#[derive(Module, Debug)]
pub struct Model<B: Backend> {
    norm: modules::LayerNorm<B>,
    conv1: modules::conv::Conv1d<B>,
    act1: nn::Relu,
    pool1: modules::pool::MaxPool1d,
    conv2: modules::conv::Conv1d<B>,
    act2: nn::Relu,
    pool2: modules::pool::MaxPool1d,
    linear: modules::Linear<B>,
}

impl<B: Backend> Model<B> {
    /// Entrée : [input_size] échantillons. Sortie : [output_size] scores (logits).
    pub fn forward(&self, x: Tensor<B, 1>) -> Tensor<B, 1> {
        let x = x.unsqueeze::<3>(); // [1, 1, 2000] : batch, canaux, temps
        let x = self.norm.forward(x); // normalise l'amplitude
        let x = self.conv1.forward(x); // [1, 4, 1998]
        let x = self.act1.forward(x);
        let x = self.pool1.forward(x); // [1, 4, 499]
        let x = self.conv2.forward(x); // [1, 8, 497]
        let x = self.act2.forward(x);
        let x = self.pool2.forward(x); // [1, 8, 124]
        // On garde la dimension batch : Linear attend [batch, entrées]
        let x = x.flatten::<2>(1, 2); // [1, 992]
        self.linear.forward(x).reshape([-1]) // [1, 3] -> [3]
    }
}

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
        // Taille après chaque couche (convolution sans padding : L - (k - 1), pooling : L / p)
        let after_conv1 = self.input_size - 2 * (self.conv1_kernel_size / 2);
        let after_pool1 = after_conv1 / self.pool1_kernel_size;
        let after_conv2 = after_pool1 - 2 * (self.conv2_kernel_size / 2);
        let after_pool2 = after_conv2 / self.pool2_kernel_size;
        let linear_input_size = after_pool2 * self.conv2_channels_count;

        Model {
            norm: modules::LayerNormConfig::new(self.input_size).init(device),
            conv1: modules::conv::Conv1dConfig::new(
                1,
                self.conv1_channels_count,
                self.conv1_kernel_size,
            )
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
