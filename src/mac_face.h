#pragma once
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    bool found;
    float x;      // 0.0 - 1.0 (sol)
    float y;      // 0.0 - 1.0 (üst)
    float width;  // 0.0 - 1.0
    float height; // 0.0 - 1.0
} MacFaceRect;

MacFaceRect mac_detect_face_in_jpeg(const unsigned char *data, unsigned long len);

#ifdef __cplusplus
}
#endif
