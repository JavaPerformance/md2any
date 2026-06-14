---
theme: light
layout: clean
aspect: a4
break_mode: off
---

<!-- layout: text-full -->

```math
\mathcal{L}_{SM} = -\frac{1}{2}\partial_\nu g^a_\mu\partial^\nu g^{a\mu}
-g_s f^{abc}\partial_\mu g^a_\nu g^{b\mu}g^{c\nu}
-\frac{1}{4}g_s^2 f^{abc}f^{ade}g^b_\mu g^c_\nu g^{d\mu}g^{e\nu}
-\partial_\nu W^+_\mu\partial^\nu W^{-\mu}-M^2W^+_\mu W^{-\mu}
-\frac{1}{2}\partial_\nu Z^0_\mu\partial^\nu Z^{0\mu}-\frac{1}{2c_w^2}M^2Z^0_\mu Z^{0\mu}
-\frac{1}{2}\partial_\mu A_\nu\partial^\mu A^\nu
-igc_w(\partial_\nu Z^0_\mu(W^{+\mu}W^{-\nu}-W^{+\nu}W^{-\mu})
-Z^0_\nu(W^+_\mu\partial^\nu W^{-\mu}-W^-_\mu\partial^\nu W^{+\mu})
+Z^0_\mu(W^+_\nu\partial^\nu W^{-\mu}-W^-_\nu\partial^\nu W^{+\mu}))
-igs_w(\partial_\nu A_\mu(W^{+\mu}W^{-\nu}-W^{+\nu}W^{-\mu})
-A_\nu(W^+_\mu\partial^\nu W^{-\mu}-W^-_\mu\partial^\nu W^{+\mu})
+A_\mu(W^+_\nu\partial^\nu W^{-\mu}-W^-_\nu\partial^\nu W^{+\mu}))
-\frac{1}{2}g^2W^+_\mu W^{-\mu}W^+_\nu W^{-\nu}
+\frac{1}{2}g^2W^+_\mu W^{+\mu}W^-_\nu W^{-\nu}
+g^2c_w^2(Z^0_\mu W^+_\nu Z^{0\mu}W^{-\nu}-Z^0_\mu Z^{0\mu}W^+_\nu W^{-\nu})
+g^2s_w^2(A_\mu W^+_\nu A^\mu W^{-\nu}-A_\mu A^\mu W^+_\nu W^{-\nu})
+g^2s_wc_w(A_\mu Z^0_\nu(W^{+\mu}W^{-\nu}+W^{-\mu}W^{+\nu})
-2A_\mu Z^{0\mu}W^+_\nu W^{-\nu})-\frac{1}{2}\partial_\mu H\partial^\mu H-2M^2\alpha_hH^2
-\partial_\mu\phi^+\partial^\mu\phi^--\frac{1}{2}\partial_\mu\phi^0\partial^\mu\phi^0
-\beta_h(\frac{2M^2}{g^2}+\frac{2M}{g}H+\frac{1}{2}(H^2+\phi^0\phi^0+2\phi^+\phi^-))
+\frac{2M^4}{g^2}\alpha_h-g\alpha_hM(H^3+H\phi^0\phi^0+2H\phi^+\phi^-)
-\frac{1}{8}g^2\alpha_h(H^4+(\phi^0)^4+4(\phi^+\phi^-)^2+4(\phi^0)^2\phi^+\phi^-
+4H^2\phi^+\phi^-+2(\phi^0)^2H^2)
-gMW^+_\mu W^{-\mu}H-\frac{1}{2}g\frac{M}{c_w^2}Z^0_\mu Z^{0\mu}H
-\frac{i}{2}g(W^+_\mu(\phi^0\partial^\mu\phi^- - \phi^-\partial^\mu\phi^0)
-W^-_\mu(\phi^0\partial^\mu\phi^+ - \phi^+\partial^\mu\phi^0))
+\frac{1}{2}g(W^+_\mu(H\partial^\mu\phi^- - \phi^-\partial^\mu H)
+W^-_\mu(H\partial^\mu\phi^+ - \phi^+\partial^\mu H))
+\frac{1}{2}g\frac{1}{c_w}Z^0_\mu(H\partial^\mu\phi^0-\phi^0\partial^\mu H)
+M(\frac{1}{c_w}Z^0_\mu\partial^\mu\phi^0+W^+_\mu\partial^\mu\phi^-+W^-_\mu\partial^\mu\phi^+)
-ig\frac{s_w^2}{c_w}MZ^0_\mu(W^{+\mu}\phi^- - W^{-\mu}\phi^+)
+igs_wMA_\mu(W^{+\mu}\phi^- - W^{-\mu}\phi^+)
-ig\frac{1-2c_w^2}{2c_w}Z^0_\mu(\phi^+\partial^\mu\phi^- - \phi^-\partial^\mu\phi^+)
+igs_wA_\mu(\phi^+\partial^\mu\phi^- - \phi^-\partial^\mu\phi^+)
-\frac{1}{4}g^2W^+_\mu W^{-\mu}(H^2+(\phi^0)^2+2\phi^+\phi^-)
-\frac{1}{8}g^2\frac{1}{c_w^2}Z^0_\mu Z^{0\mu}(H^2+(\phi^0)^2+2(2s_w^2-1)^2\phi^+\phi^-)
-\frac{1}{2}g^2\frac{s_w}{c_w}Z^0_\mu\phi^0(W^{+\mu}\phi^-+W^{-\mu}\phi^+)
-\frac{i}{2}g^2\frac{s_w}{c_w}Z^0_\mu H(W^{+\mu}\phi^- - W^{-\mu}\phi^+)
+\frac{1}{2}g^2s_wA_\mu\phi^0(W^{+\mu}\phi^-+W^{-\mu}\phi^+)
+\frac{i}{2}g^2s_wA_\mu H(W^{+\mu}\phi^- - W^{-\mu}\phi^+)
-g^2\frac{s_w}{c_w}(2c_w^2-1)Z^0_\mu A^\mu\phi^+\phi^- - g^2s_w^2A_\mu A^\mu\phi^+\phi^-
+\frac{i}{2}g_s\lambda^a_{ij}(\bar{q}_i\gamma^\mu q_j)g^a_\mu
-\bar{e}^\lambda(\gamma\partial+m_e^\lambda)e^\lambda-\bar{\nu}^\lambda(\gamma\partial+m_\nu^\lambda)\nu^\lambda
-\bar{u}^\lambda_j(\gamma\partial+m_u^\lambda)u^\lambda_j-\bar{d}^\lambda_j(\gamma\partial+m_d^\lambda)d^\lambda_j
+igs_wA_\mu(-(\bar{e}^\lambda\gamma^\mu e^\lambda)
+\frac{2}{3}(\bar{u}^\lambda_j\gamma^\mu u^\lambda_j)-\frac{1}{3}(\bar{d}^\lambda_j\gamma^\mu d^\lambda_j))
+\frac{ig}{4c_w}Z^0_\mu(\bar{\nu}^\lambda\gamma^\mu(1+\gamma^5)\nu^\lambda
+\bar{e}^\lambda\gamma^\mu(4s_w^2-1-\gamma^5)e^\lambda
+\bar{d}^\lambda_j\gamma^\mu(\frac{4}{3}s_w^2-1-\gamma^5)d^\lambda_j
+\bar{u}^\lambda_j\gamma^\mu(1-\frac{8}{3}s_w^2+\gamma^5)u^\lambda_j)
+\frac{ig}{2\sqrt{2}}W^+_\mu((\bar{\nu}^\lambda\gamma^\mu(1+\gamma^5)U^{lep}_{\lambda\kappa}e^\kappa)
+(\bar{u}^\lambda_j\gamma^\mu(1+\gamma^5)C_{\lambda\kappa}d^\kappa_j))
+\frac{ig}{2\sqrt{2}}W^-_\mu((\bar{e}^\kappa U^{lep\dagger}_{\kappa\lambda}\gamma^\mu(1+\gamma^5)\nu^\lambda)
+(\bar{d}^\kappa_jC^\dagger_{\kappa\lambda}\gamma^\mu(1+\gamma^5)u^\lambda_j))
+\frac{ig}{2M\sqrt{2}}\phi^+(-m_e^\kappa\bar{\nu}^\lambda U^{lep}_{\lambda\kappa}(1-\gamma^5)e^\kappa
+m_\nu^\lambda\bar{\nu}^\lambda U^{lep}_{\lambda\kappa}(1+\gamma^5)e^\kappa)
+\frac{ig}{2M\sqrt{2}}\phi^-(m_e^\lambda\bar{e}^\lambda U^{lep\dagger}_{\lambda\kappa}(1+\gamma^5)\nu^\kappa
-m_\nu^\kappa\bar{e}^\lambda U^{lep\dagger}_{\lambda\kappa}(1-\gamma^5)\nu^\kappa)
-\frac{g m_\nu^\lambda}{2M}H(\bar{\nu}^\lambda\nu^\lambda)-\frac{g m_e^\lambda}{2M}H(\bar{e}^\lambda e^\lambda)
+\frac{ig m_\nu^\lambda}{2M}\phi^0(\bar{\nu}^\lambda\gamma^5\nu^\lambda)
-\frac{ig m_e^\lambda}{2M}\phi^0(\bar{e}^\lambda\gamma^5e^\lambda)
-\frac{1}{4}\bar{\nu}_\lambda M^R_{\lambda\kappa}(1-\gamma^5)\hat{\nu}_\kappa
-\frac{1}{4}\bar{\nu}_\lambda M^R_{\lambda\kappa}(1-\gamma^5)\hat{\nu}_\kappa
+\frac{ig}{2M\sqrt{2}}\phi^+(-m_d^\kappa\bar{u}^\lambda_jC_{\lambda\kappa}(1-\gamma^5)d^\kappa_j
+m_u^\lambda\bar{u}^\lambda_jC_{\lambda\kappa}(1+\gamma^5)d^\kappa_j)
+\frac{ig}{2M\sqrt{2}}\phi^-(m_d^\lambda\bar{d}^\lambda_jC^\dagger_{\lambda\kappa}(1+\gamma^5)u^\kappa_j
-m_u^\kappa\bar{d}^\lambda_jC^\dagger_{\lambda\kappa}(1-\gamma^5)u^\kappa_j)
-\frac{g m_u^\lambda}{2M}H(\bar{u}^\lambda_j u^\lambda_j)-\frac{g m_d^\lambda}{2M}H(\bar{d}^\lambda_jd^\lambda_j)
+\frac{ig m_u^\lambda}{2M}\phi^0(\bar{u}^\lambda_j\gamma^5u^\lambda_j)
-\frac{ig m_d^\lambda}{2M}\phi^0(\bar{d}^\lambda_j\gamma^5d^\lambda_j)
+\bar{G}^a\partial^2G^a+g_sf^{abc}\partial_\mu\bar{G}^aG^bg^c_\mu
+X^+(\partial^2-M^2)X^+ + X^-(\partial^2-M^2)X^- + X^0(\partial^2-\frac{M^2}{c_w^2})X^0+\bar{Y}\partial^2Y
+igc_wW^+_\mu(\partial^\mu X^0X^- - \partial^\mu X^+X^0)
+igs_wW^+_\mu(\partial^\mu YX^- - \partial^\mu X^+Y)
+igc_wW^-_\mu(\partial^\mu X^-X^0 - \partial^\mu X^0X^+)
+igs_wW^-_\mu(\partial^\mu X^-Y - \partial^\mu YX^+)
+igc_wZ^0_\mu(\partial^\mu X^+X^+ - \partial^\mu X^-X^-)
+igs_wA_\mu(\partial^\mu X^+X^+ - \partial^\mu X^-X^-)
-\frac{1}{2}gM(\bar{X}^+X^+H+\bar{X}^-X^-H+\frac{1}{c_w^2}\bar{X}^0X^0H)
+\frac{1-2c_w^2}{2c_w}igM(\bar{X}^+X^0\phi^- - \bar{X}^-X^0\phi^+)
+\frac{1}{2c_w}igM(\bar{X}^0X^-\phi^+ - \bar{X}^0X^+\phi^-)
+igMs_w(\bar{X}^0X^-\phi^+ - \bar{X}^0X^+\phi^-)
+\frac{i}{2}gM(\bar{X}^+X^+\phi^0 - \bar{X}^-X^-\phi^0).
```
