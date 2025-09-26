import re
import csv
import pandas as pd
import matplotlib.pyplot as plt
from enum import StrEnum

RESULT_CSV_PATH = "./data/bench_result.csv"
class Metric(StrEnum):
    SIZE = "size",
    SCAN_TIME = "scan_time",
    
def get_result_csv():
    data = {}
    def process_log(log_file_name: str, parquet_pattern: re.Pattern, vortex_pattern: re.Pattern, lance_pattern: re.Pattern, metric: Metric):
        with open(log_file_name, 'r') as file:
            log_data = file.read()
        workload_pattern = re.compile(r'/parquet/(\w+).parquet')
        current_workload, current_parquet_metric, current_vortex_metric, current_lance_metric = None, None, None, None

        for line in log_data.strip().split('\n'):
            workload_match = workload_pattern.search(line)
            parquet_match = parquet_pattern.search(line)
            vortex_match = vortex_pattern.search(line)
            lance_match = lance_pattern.search(line)
            
            if workload_match:
                current_workload = workload_match.group(1)
            if parquet_match:
                current_parquet_metric = parquet_match.group(1)
            if vortex_match:
                current_vortex_metric = vortex_match.group(1)
            if lance_match:
                current_lance_metric = lance_match.group(1)
            if current_workload and current_parquet_metric and current_vortex_metric and current_lance_metric:
                data.setdefault(current_workload, {})
                data[current_workload]['workload'] = current_workload
                if metric == Metric.SIZE:
                    data[current_workload]['parquet_size'] = current_parquet_metric
                    data[current_workload]['vortex_size'] = current_vortex_metric
                    data[current_workload]['lance_size'] = current_lance_metric
                elif metric == Metric.SCAN_TIME:
                    data[current_workload]['parquet_scan_time'] = current_parquet_metric
                    data[current_workload]['vortex_scan_time'] = current_vortex_metric
                    data[current_workload]['lance_scan_time'] = current_lance_metric
                else:
                    assert False
                current_workload, current_parquet_metric, current_vortex_metric, current_lance_metric = None, None, None, None

    parquet_pattern = re.compile(r'Parquet size: ([\d.]+) MB, \d+B')
    vortex_pattern = re.compile(r'Vortex size: ([\d.]+) MB, \d+B')
    lance_pattern = re.compile(r'Lance directory aggregate size: ([\d.]+) MB, \d+B')
    process_log('compress_bench.log', parquet_pattern, vortex_pattern, lance_pattern, Metric.SIZE)

    parquet_pattern = re.compile(r'Reading parquet file took ([\d.]+)ms')
    vortex_pattern = re.compile(r'Reading vortex file took ([\d.]+)ms')
    lance_pattern = re.compile(r'Reading lance file took ([\d.]+)ms')
    process_log('scan_bench.log', parquet_pattern, vortex_pattern, lance_pattern, Metric.SCAN_TIME)
    
    with open(RESULT_CSV_PATH, 'w', newline='') as csvfile:
        fieldnames = ['workload', 'parquet_size', 'vortex_size', 'lance_size',
                      'lance_scan_time', 'parquet_scan_time', 'vortex_scan_time']
        writer = csv.DictWriter(csvfile, fieldnames=fieldnames)
        writer.writeheader()
        for wl in data:
            writer.writerow(data[wl])

def plot():
    df = pd.read_csv(RESULT_CSV_PATH)
    def subplot(metric: Metric):
        fig, ax = plt.subplots(figsize=(10, 6))
        bar_width = 0.2
        # the label locations
        r1 = range(len(df['workload']))
        r2 = [x + bar_width for x in r1]
        r3 = [x + bar_width for x in r2]

        # Make the plot
        ax.bar(r1, df[f'parquet_{metric}'], width=bar_width, label='Parquet', edgecolor='grey')
        ax.bar(r2, df[f'vortex_{metric}'], width=bar_width, label='Vortex', edgecolor='grey')
        ax.bar(r3, df[f'lance_{metric}'], width=bar_width, label='Lance', edgecolor='grey')

        # Add some text for labels, title and custom x-axis tick labels, etc.
        ax.set_xlabel('Workload')
        if metric == Metric.SIZE:
            ax.set_ylabel(f'Size (MB)')
        elif metric == Metric.SCAN_TIME:
            ax.set_ylabel(f'Scan Time (ms)')
        else:
            assert False
        ax.set_title('CFB bench result')
        ax.set_xticks([r + bar_width for r in range(len(df['workload']))])
        ax.set_xticklabels(df['workload'])

        # Create legend & Show graphic
        ax.legend()

        # Rotate the x-axis labels for better readability
        plt.xticks(rotation=45)

        # Save the figure to a file
        plt.savefig(f'./data/{metric}_comparison_bar_graph.png', bbox_inches='tight')
    subplot(Metric.SIZE)
    subplot(Metric.SCAN_TIME)

if __name__ == '__main__':
    get_result_csv()
    plot()