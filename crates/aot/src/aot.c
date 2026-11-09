/* The AOT runtime shim: the host functions and the process entry point.
 *
 * `pietrs build` compiles this file with the emitted object and links both.
 * The step limit is passed as `-DPIET_AOT_MAX_STEPS=...`; the default below
 * keeps the file valid and complete when opened on its own. The Context layout
 * must match codegen's `Context`; the static assertions fail the build if it
 * ever drifts.
 */

#if defined(_WIN32) && !defined(_CRT_SECURE_NO_WARNINGS)
#define _CRT_SECURE_NO_WARNINGS
#endif

#include <inttypes.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    int64_t *stack;
    uint64_t capacity;
    uint64_t sp;
    uint64_t steps;
    uint64_t max_steps;
    uint32_t reason;
} Context;

_Static_assert(sizeof(Context) == 48, "Context size");
_Static_assert(offsetof(Context, stack) == 0, "stack offset");
_Static_assert(offsetof(Context, capacity) == 8, "capacity offset");
_Static_assert(offsetof(Context, sp) == 16, "sp offset");
_Static_assert(offsetof(Context, steps) == 24, "steps offset");
_Static_assert(offsetof(Context, max_steps) == 32, "max_steps offset");
_Static_assert(offsetof(Context, reason) == 40, "reason offset");

extern void piet_main(Context *context);

static const uint64_t PIET_STACK_CAPACITY = 1u << 20;

#ifndef PIET_AOT_MAX_STEPS
#define PIET_AOT_MAX_STEPS UINT64_C(18446744073709551615)
#endif

static const uint64_t PIET_MAX_STEPS = PIET_AOT_MAX_STEPS;

enum { REASON_TERMINATED = 0, REASON_STEP_LIMIT = 1, REASON_OVERFLOW = 2 };

static void push(Context *context, int64_t value) {
    if (context->sp < context->capacity) {
        context->stack[context->sp] = value;
        context->sp += 1;
    } else {
        context->reason = REASON_OVERFLOW;
    }
}

static int pop(Context *context, int64_t *value) {
    if (context->sp == 0) {
        return 0;
    }
    context->sp -= 1;
    *value = context->stack[context->sp];
    return 1;
}

static int pop_two(Context *context, int64_t *second, int64_t *top) {
    if (context->sp < 2) {
        return 0;
    }
    *top = context->stack[--context->sp];
    *second = context->stack[--context->sp];
    return 1;
}

void piet_out_char(Context *context) {
    int64_t value;
    if (!pop(context, &value)) {
        return;
    }
    if (value < 0 || value > 0x10FFFF) {
        return;
    }
    uint32_t code = (uint32_t)value;
    if (code >= 0xD800 && code <= 0xDFFF) {
        return;
    }
    char bytes[4];
    size_t length;
    if (code < 0x80) {
        bytes[0] = (char)code;
        length = 1;
    } else if (code < 0x800) {
        bytes[0] = (char)(0xC0 | (code >> 6));
        bytes[1] = (char)(0x80 | (code & 0x3F));
        length = 2;
    } else if (code < 0x10000) {
        bytes[0] = (char)(0xE0 | (code >> 12));
        bytes[1] = (char)(0x80 | ((code >> 6) & 0x3F));
        bytes[2] = (char)(0x80 | (code & 0x3F));
        length = 3;
    } else {
        bytes[0] = (char)(0xF0 | (code >> 18));
        bytes[1] = (char)(0x80 | ((code >> 12) & 0x3F));
        bytes[2] = (char)(0x80 | ((code >> 6) & 0x3F));
        bytes[3] = (char)(0x80 | (code & 0x3F));
        length = 4;
    }
    fwrite(bytes, 1, length, stdout);
    fflush(stdout);
}

void piet_out_number(Context *context) {
    int64_t value;
    if (pop(context, &value)) {
        printf("%" PRId64, value);
        fflush(stdout);
    }
}

void piet_in_char(Context *context) {
    int first = getchar();
    if (first == EOF) {
        return;
    }
    uint32_t code;
    int extra;
    if (first < 0x80) {
        code = (uint32_t)first;
        extra = 0;
    } else if ((first & 0xE0) == 0xC0) {
        code = (uint32_t)(first & 0x1F);
        extra = 1;
    } else if ((first & 0xF0) == 0xE0) {
        code = (uint32_t)(first & 0x0F);
        extra = 2;
    } else if ((first & 0xF8) == 0xF0) {
        code = (uint32_t)(first & 0x07);
        extra = 3;
    } else {
        return;
    }
    for (int i = 0; i < extra; i += 1) {
        int next = getchar();
        if (next == EOF || (next & 0xC0) != 0x80) {
            return;
        }
        code = (code << 6) | (uint32_t)(next & 0x3F);
    }
    if (code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
        return;
    }
    push(context, (int64_t)code);
}

void piet_in_number(Context *context) {
    int64_t value;
    if (scanf("%" SCNd64, &value) == 1) {
        push(context, value);
    }
}

void piet_roll(Context *context) {
    int64_t depth;
    int64_t rolls;
    if (!pop_two(context, &depth, &rolls)) {
        return;
    }
    if (depth < 0 || (uint64_t)depth > context->sp || depth <= 1) {
        return;
    }
    uint64_t count = (uint64_t)depth;
    uint64_t start = context->sp - count;
    int64_t shift = rolls % (int64_t)count;
    if (shift < 0) {
        shift += (int64_t)count;
    }
    for (int64_t i = 0; i < shift; i += 1) {
        int64_t last = context->stack[context->sp - 1];
        memmove(&context->stack[start + 1], &context->stack[start],
                (size_t)(count - 1) * sizeof(int64_t));
        context->stack[start] = last;
    }
}

int main(void) {
    int64_t *stack = (int64_t *)calloc((size_t)PIET_STACK_CAPACITY, sizeof(int64_t));
    if (stack == NULL) {
        fputs("pietrs: cannot allocate the stack\n", stderr);
        return 1;
    }
    Context context = {
        .stack = stack,
        .capacity = PIET_STACK_CAPACITY,
        .sp = 0,
        .steps = 0,
        .max_steps = PIET_MAX_STEPS,
        .reason = REASON_TERMINATED,
    };
    piet_main(&context);
    int status = 0;
    switch (context.reason) {
    case REASON_TERMINATED:
        status = 0;
        break;
    case REASON_STEP_LIMIT:
        fprintf(stderr, "pietrs: step limit reached after %" PRIu64 " steps\n",
                context.steps);
        status = 1;
        break;
    case REASON_OVERFLOW:
        fputs("pietrs: the stack overflowed\n", stderr);
        status = 1;
        break;
    default:
        status = 1;
        break;
    }
    free(stack);
    return status;
}
