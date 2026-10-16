#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--version") == 0) {
        puts("Difftastic 0.71.0");
        return 0;
    }
    puts("{\"status\":\"changed\",\"language\":\"JavaScript\",\"aligned_lines\":[[0,0]],\"chunks\":[[{\"lhs\":{\"changes\":[{\"content\":\"1\"}]},\"rhs\":{\"changes\":[{\"content\":\"2\"}]}}]]}");
    return 0;
}
