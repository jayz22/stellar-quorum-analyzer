## Disclaimer
The following derivation of the formulas used in this work is based on Giuliano(@nano-o)'s methodology. Giuliano is the original author and rightful credit holder of this approach. This derivation serves two primary purposes:
- Educational. To deepen my, as well as any interested users' understanding of the implemented method by working through its details.
- To provide explicit documentation of the formulas being implemented.

This derivation is likely not perfect and may contain bugs. If in question, please refer to the actual code for the source of truth, or file a bug report.


## Derivation
Consider A two-quorum set $Q = \{A, B\}$ and a set of validators $V = \{v_0 .. v_{N-1}\}$

**Objective**: To prove that for any two validators, their quorums intersect.

**Challenge**: Multiple quorums can exist depending on which slice in the qset is selected for each validator. It is insufficient to find two intersecting quorums; instead, we must demonstrate that no qset path leads to two non-intersecting quorums. This can be achieved by solving for one non-intersecting quorum configuration. If found, it proves a contradiction to the intersection property.

### Constrains:

#### 1. **Non-Empty Quorums**
Both quorums A and B must be non-empty:

$$
\left(\bigvee_{i=0}^{N-1}  Av_i\right)\bigwedge\left(\bigvee_{i=0}^{N-1}  Bv_i\right)
$$
This is already in CNF form. We generate `N` (`N` is the number of eligible validators) constraints corresponding to each clause in the parentheses.

#### 2. **Non-Intersecting Quorums**. 
Quorum A and B do not intersect (i.e., no validator exists in both quorums)
$$
\bigwedge_{i=0}^{i=N-1} \left(\neg Av_i \bigvee \neg Bv_i \right)
$$
This is also in CNF form. We add N constraints, one for each eligible validator.

#### 3. Quorum Satisfaction by Transitive QSets
For each node in the graph (validator or transitive qset), satisfaction of the quorum implies that the threshold of its successors is satisfied by the quorum. Formally:

$$
\bigwedge_{i=0}^{V-1} \left( Aq_i \implies \Phi^A_i  \right) \bigwedge \left( Bq_i \implies \Phi^B_i \right)
$$

where $\Phi^A_i$ represents the logic that $q_i$'s quorum condition is satisfied by quorum $A$ , and $V$ is the number of vertices in the graph.

using $a \implies b  \equiv \neg a \bigvee b$, the formula expands to:

$$
\bigwedge_{i=0}^{V-1} \left( \neg Aq_i \bigvee \Phi^A_i \right) \bigwedge \left( \neg Bq_i \bigvee \Phi^B_i \right)
$$

This is not CNF form. So the remaining task is to expand this part, and transform it using Teistin transformation into CNF form.

##### Expansion of $\Phi_i$

Define $\Pi^i$ the combinatorial set of $q_i$'s immediate successors:
$$
 \Pi^i = \binom{\text{num\_successors}(q_i)}{\text{threshold}(q_i)}
$$

$$
\Phi_i \equiv \bigvee_{j=0}^{J-1} \bigwedge_{k=0}^{K-1} \Pi^i_{j,k}
$$

Here $j$ loops over all the subsets in $\Pi$ and $k$ loops over elements within each subset. This is saying that at least one `qset` must have all its members be part of the quorum (I've emitted the quorum label $A$ otherwise the script gets too messy).

However, this is in DNF form. To convert to CNF, we apply Tseitin transformation.

##### Applying Tseitin Transformation
Introduce a new variable $\xi^i_j$ for each inner AND relation:

$$
\xi^i_j \leftrightarrow \bigwedge_{k=0}^{K-1} \Pi^i_{j,k}
$$

expand the equivalence $a \leftrightarrow b \equiv \left(a \implies b\right) \bigwedge \left( b \implies a\right)$, and further expand it by applying the distribution law:

$$
 \left( \bigwedge_{k=0}^{K-1} \left( \neg \xi^i_j \bigvee \Pi^i_{j,k} \right) \right) \bigwedge \left( \xi^i_j \left( \bigvee_{k=0}^{K-1}\neg \Pi^i_{j,k} \right) \right)
$$

All we've done above is introducing new logic gates $\xi$ and making it equivalent to the inner gates wired (AND'ed) together, and this is in CNF form.

This must be done for all $j$, i.e. every slice in $q_i$'s qset. In addition, we must also wire in the outer OR relation 
$\bigvee_{j=0}^{J-1} \xi^i_{j}$. The combined relation is follows.

$$
\Phi_i \equiv \bigwedge_{j=0}^{j=J-1} \left( \left( \bigwedge_{k=0}^{K-1} \left( \neg \xi^i_j \bigvee \Pi^i_{j,k} \right) \right) \bigwedge \left( \xi^i_j \left( \bigvee_{k=0}^{K-1}\neg \Pi^i_{j,k} \right) \right) \right) \bigwedge \left( \bigvee_{j=0}^{J-1} \xi^{i}_{j} \right)
$$

Reintroduce the antecedent constrain $\neg Aq_i \bigvee \Phi^A_i$, applying the distribution law to ensure $\neg Aq_i$ is OR'ed with each term inside $\Phi^A_i$:

$$
qsat_i^A = \bigwedge_{j=0}^{J-1} \left( \left( \bigwedge_{k=0}^{K-1} \left(\neg Aq_i \bigvee \neg \xi^i_j \bigvee \Pi^i_{j,k} \right) \right) \bigwedge \left( \neg Aq_i \bigvee \xi^i_j \left( \bigvee_{k=0}^{K-1}\neg \Pi^i_{j,k} \right) \right) \right) \bigwedge \left(  \neg Aq_i \bigvee_{j=0}^{J-1} \xi^{i}_{j} \right)
$$

In the end we combine this into the master constrain for all vertices `V`
$$
\bigwedge_{i=0}^{V-1} qsat_i^A \bigwedge qsat_i^B
$$
and we get our third constrain in CNF form.