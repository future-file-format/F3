# Analyze a WAT file to see how many functions and LoC

import re

def parse_wat_file(file_path):
    with open(file_path, 'r') as file:
        lines = file.readlines()
    
    functions = {}
    current_function = None
    line_count = 0

    for line in lines:
        line = line.strip()
        func_match = re.match(r'\(func (\$[^\s]*)', line)
        
        if func_match:
            if current_function:
                functions[current_function] = line_count

            current_function = func_match.group(1)
            line_count = 1  # Start line count for the new function
        elif current_function:
            if line != '':
                line_count += 1

    if current_function:
        functions[current_function] = line_count
    
    return functions

def main():
    file_path = '/home/xinyu/fff-devel/target/wasm32-unknown-unknown/wizard-nostrip/fff_ude_example_noop.wat'
    # file_path = '/home/xinyu/fff-devel/target/wasm32-wasip1/wizard-nostrip/fff_ude_example_noop.wat'
    functions = parse_wat_file(file_path)
    
    # Sort functions by LOC in descending order
    sorted_functions = sorted(functions.items(), key=lambda x: x[1], reverse=True)
    
    for function_name, loc in sorted_functions:
        print(f"Function: {function_name}, Lines of Code: {loc}")

if __name__ == "__main__":
    main()