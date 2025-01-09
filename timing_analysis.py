import pandas as pd
import matplotlib.pyplot as plt

# Load the data
file_path = './benchmark_results_table.csv'
df = pd.read_csv(file_path, sep=r'\s{2,}', engine='python')

# Filter cases where at least one solver took >= 1000 usecs
filtered_df = df[df['solve_time (usecs)'] >= 1000]

# Group by 'file_name' and 'solver_name' to find unique test cases where at least one solver's time >= 1000
cases_with_slow_solve = filtered_df['file_name'].unique()
filtered_cases = df[df['file_name'].isin(cases_with_slow_solve)]

# Pivot the data for plotting
pivot_data = filtered_cases.pivot(index='file_name', columns='solver_name', values='solve_time (usecs)')

# Plot with log scale for y-axis, explicit labels on x-axis, and no line connections.
plt.figure(figsize=(14, 7))

# Plot with log scale and markers only
for solver in pivot_data.columns:
    plt.scatter(pivot_data.index, pivot_data[solver], label=solver)

plt.yscale('log')  # Set y-axis to logarithmic scale
plt.title('Comparison of Solve Times Across Solvers (Log Scale)')
plt.xlabel('Test Case')
plt.ylabel('Solve Time (usecs)')
plt.xticks(ticks=range(len(pivot_data.index)), labels=pivot_data.index, rotation=45, ha='right')
plt.legend(title='Solver Name')
plt.grid(True, which="both", linestyle='--', linewidth=0.5)
plt.tight_layout()
plt.savefig("plot_solve_times.png", dpi=300, bbox_inches='tight')
plt.show()

# Filter cases where at least one solver's setup time >= 5000 usecs
filtered_setup_df = df[df['setup_time (usecs)'] >= 4000]

# Group by 'file_name' and 'solver_name' to find unique test cases
cases_with_slow_setup = filtered_setup_df['file_name'].unique()
filtered_setup_cases = df[df['file_name'].isin(cases_with_slow_setup)]

# Pivot the data for plotting setup times
pivot_setup_data = filtered_setup_cases.pivot(index='file_name', columns='solver_name', values='setup_time (usecs)')

# Plotting setup times with the same requested adjustments
plt.figure(figsize=(14, 7))

# Plot with log scale and markers only for setup times
for solver in pivot_setup_data.columns:
    plt.scatter(pivot_setup_data.index, pivot_setup_data[solver], label=solver)

plt.yscale('log')  # Set y-axis to logarithmic scale
plt.title('Comparison of Setup Times Across Solvers (Log Scale)')
plt.xlabel('Test Case')
plt.ylabel('Setup Time (usecs)')
plt.xticks(ticks=range(len(pivot_setup_data.index)), labels=pivot_setup_data.index, rotation=45, ha='right')
plt.legend(title='Solver Name')
plt.grid(True, which="both", linestyle='--', linewidth=0.5)
plt.tight_layout()
plt.savefig("plot_setup_times.png", dpi=300, bbox_inches='tight')
plt.show()
