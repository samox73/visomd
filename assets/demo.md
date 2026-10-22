# Fourier transforms and the Gaussian

A function $f(x)$ and its transform $\hat f(k)$ carry the same information. With the symmetric convention,

$$
\hat f(k) = \frac{1}{\sqrt{2\pi}} \int_{-\infty}^{\infty} f(x) \, e^{-ikx} \, dx, \qquad f(x) = \frac{1}{\sqrt{2\pi}} \int_{-\infty}^{\infty} \hat f(k) \, e^{ikx} \, dk,
$$

and the Gaussian $e^{-x^2/2\sigma^2}$ is its own transform, with width $1/\sigma$. Three facts worth remembering:

- *Uncertainty:* $\sigma_x \sigma_k \geq \tfrac{1}{2}$, with equality **only** for Gaussians.
- *Convolution:* $\widehat{f * g} = \sqrt{2\pi} \, \hat f \, \hat g$, so smoothing in $x$ is a multiplication in $k$.
- *Derivatives:* $\widehat{f'}(k) = ik \hat f(k)$, which turns a differential equation into algebra:
  1. solve for $\hat f$,
  2. transform back, often with the residue theorem.

| function $f(x)$ | transform $\hat f(k)$ | notes |
|:--|:-:|--:|
| $e^{-x^2/2}$ | $e^{-k^2/2}$ | self-dual |
| $\theta(1 - \lvert x \rvert)$ | $\sqrt{2/\pi} \, \sin k / k$ | a sinc |
| $\delta(x - a)$ | $e^{-ika} / \sqrt{2\pi}$ | pure phase |

> Euler's identity $e^{i\pi} + 1 = 0$ is just $e^{ikx}$ sampled at $k = \pi$, $x = 1$.

```python
x = np.linspace(-10, 10, 4096)
f_hat = np.fft.fftshift(np.fft.fft(np.exp(-x**2 / 2)))
```
