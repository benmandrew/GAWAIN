# Simulink-based benchmark sources

Checked on 2026-10-23. Anything marked "unverified" I could not confirm from a downloaded file or an API response. Verdict in one line: only LMCPS is worth a second look, and only as a small source of hand-translated cases; ARCH-COMP and parametric STL mining do not fit a discrete model checker.

## 1. Lockheed Martin Cyber-Physical Challenges (LMCPS)

Sources. The NASA report page resolves (HTTP 200): https://ntrs.nasa.gov/citations/20205000433. I read the earlier NASA paper on the same work, "Bridging the Gap Between Requirements and Model Analysis" (https://ntrs.nasa.gov/api/citations/20200002241/downloads/20200002241.pdf), for the counts. I did not read the 2020 report itself, so its figures are unverified.

Download. The models and requirements are at https://github.com/hbourbouh/lm_challenges (resolves; "Cyber-Physical V&V Challenges for the Evaluation of State of the Art Model Checkers"). The `original_models` folder holds ten problems, each with a Simulink `.mdl` file and a `.docx` of natural-language requirements. The `FRET_CoCoSim` folder holds the FRET-formalised versions. The paper also links a Google Drive copy, which I did not try.

Licence. The repository has no licence file (GitHub reports none). The README says "Copyright 2016, Lockheed Martin Corporation. All rights reserved". I therefore cannot assume it may be redistributed. Do not copy the files into this repository without asking the owners. Re-fetch from the URL instead.

Counts (Table 1 of the NASA paper, measured with Kind2 and Simulink Design Verifier, 2 hour timeout). There are 74 requirements over 10 models, 69 formalised, 64 analysed. Kind2 found 23 valid, 23 invalid and 18 undecided. SLDV found 21 valid, 12 invalid and 31 undecided. So about 23 requirements are known to be violated by the model, and 18 to 31 more are undecided. Some violations may come from ambiguous wording (the paper's FSM-001 needed two readings, and both failed), so a violated requirement is not always a case where a weaker requirement was intended.

Formula language. The paper formalises requirements in FRETISH (structured English), which FRET turns into past-time metric LTL over discrete steps. Time bounds are in steps, so the formulas are close to MTL on a discrete clock.

Running without MATLAB. Not possible with the files as given. The models are Simulink `.mdl` files. I found no Python or C port (unverified; searched only briefly). The route the paper uses is Simulink to Lustre (CoCoSim, LustreC) and then Kind2. A Lustre translation could in principle be turned into an SMV model by hand, but that needs MATLAB for the first step.

Discrete model checker. Partly. The models are discrete-time and the properties are bounded, so a nuXmv encoding is plausible for the small ones (triplex monitor, FSM, Tustin integrator, system safety monitor). The large ones use floating-point, non-linear and neural-network blocks, which nuXmv cannot take without abstraction. Whether FRET exports to nuXmv directly is unverified; the documented path is CoCoSpec and Kind2.

## 2. ARCH-COMP falsification category

Sources. The 2019 report resolves (HTTP 200): https://www.sosy-lab.org/research/pub/2019-ARCH.Category_Report_Falsification.pdf. A 2024 report also exists (listed at https://research.abo.fi/fi/publications/arch-comp-2024-category-report-falsification/); I did not open it, so later benchmark changes are unverified.

Download. Participant data (not clean benchmark files) are in the GitLab repository https://gitlab.com/goranf/ARCH-COMP (resolves). It has a `2019/FALS` folder with subfolders for the tools: MCTS, S-TaLiRo, breach, falsify, falstar and shared. The 2023 folder has no falsification folder, so later editions may live elsewhere (unverified). The ARCH site is https://cps-vo.org/group/ARCH (resolves). The benchmark models come from papers and tool repositories, for example Breach at https://github.com/decyphir/breach (resolves) and FalStar at https://github.com/ERATOMMSD/falstar (resolves).

Licence. Not checked for the GitLab data (unverified). Breach carries a LICENSE file whose type GitHub does not recognise ("Other"). FalStar is BSD-3-Clause.

Counts. Counting the tables in the 2019 report, there are 24 requirements over 7 models: AT 9 (AT1, AT2, AT51 to AT54, AT6a to AT6c), AFC 3, NN 1, WT 4, CC 5, F16 1, SC 1. The report states that they were chosen so that falsification is possible but hard. Table 1 shows that tools falsified nearly all of them in 50 runs, with exceptions such as SC, which only one tool falsified (50/50, while the others scored 0/50). So the large majority are violated, but this is evidence from tools and not a proof. The exact number violated by every tool is unverified.

Formula language. STL (the report says MTL or STL), with dense real time and real-valued signals, for example `G[0,20] v < 120`.

Running without MATLAB. Mostly no. The models are Simulink/Stateflow. Python tooling exists: Psi-TaLiRo is a BSD-3-Clause Python counterpart of S-TaLiRo that was reported to run ARCH benchmarks, but those models still run through Simulink (reported by search results, unverified). FalCAuN reports that F16 needs a large MATLAB wrapper and that "sabo" is Python (search result, unverified). I found no pure Python or C port of the main models.

Discrete model checker. No. The plant models are continuous hybrid systems with real-valued signals, and the properties use real thresholds. A fixed-step trace could be exported and turned into a finite model, but that is a trace and not a model of the system, so a model checker proves nothing about it.

## 3. Parametric STL mining

Sources. Jin et al., "Mining Requirements from Closed-Loop Control Models", IEEE TCAD 34(11), 2015: page resolves (HTTP 200) at https://people.eecs.berkeley.edu/~sseshia/pubs/b2hd-jin-tcad15.html. The Hoxha link https://archive.cps-vo.org/node/50635 resolves with HTTP 200 for a command-line fetch, but a browser-style fetch returned "Request Rejected", so I could not read it (unverified). A web search shows the Hoxha, Abbas and Fainekos paper "Benchmarks for Temporal Logic Requirements for Automotive Systems" is in the ARCH14-15 proceedings (EPiC Series vol. 34, pp. 25 to 30), https://easychair.org/publications/paper/4bfq. It uses two Simulink models: the Automatic Transmission and the Fault-Tolerant Fuel Control System. I did not confirm its requirement count (a figure of 27 was suggested to me, which is unverified).

Download. The Jin et al. case studies are an automatic transmission controller, an air-fuel controller and an industrial diesel airpath controller. The abstract does not say that the models are public, and the last is described as an industrial prototype, so it is probably not available (unverified). Mining is implemented in Breach (MATLAB). I did not find a dataset of mined requirements.

Licence. Unverified. Breach is the only tool I checked (see above).

Counts. Not applicable as a fixed list. The method produces requirements that the traces satisfy, so by construction it gives a satisfied formula and not a violated one. The paper reports one bug found in the airpath controller.

Formula language. Parametric STL (formulas with parameters in place of thresholds and time bounds).

Running without MATLAB. No. Simulink models and a MATLAB toolbox.

Discrete model checker. No, for the same reasons as ARCH-COMP. The idea is useful, though: parametric STL mining is a form of weakening, because it searches for the strongest parameter values that still hold. It gives a method to compare against, not a benchmark.

## Summary for the plan

LMCPS is usable in a limited way: real, human-written requirements, about 23 known violated, discrete time. The cost is a licence that forbids copying, MATLAB for the translation, and only a few small models that fit nuXmv. ARCH-COMP and parametric STL are unusable with nuXmv or SPIN because they are continuous Simulink systems under STL. Keep them only as related work.
