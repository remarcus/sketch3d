"""Encode a numbered PNG frame sequence as H.264 MP4 through Blender's FFmpeg."""

import os
import sys

import bpy


def arguments():
    try:
        separator = sys.argv.index("--")
    except ValueError as error:
        raise RuntimeError("Expected frame, output, size, rate, and count arguments after --") from error
    values = sys.argv[separator + 1 :]
    if len(values) != 6:
        raise RuntimeError("Expected six encoder arguments")
    frame_dir, output_path, width, height, fps, frame_count = values
    return (
        os.path.abspath(frame_dir),
        os.path.abspath(output_path),
        int(width),
        int(height),
        int(fps),
        int(frame_count),
    )


def main():
    frame_dir, output_path, width, height, fps, frame_count = arguments()
    frame_names = [f"frame_{index:06}.png" for index in range(1, frame_count + 1)]
    for name in frame_names:
        if not os.path.isfile(os.path.join(frame_dir, name)):
            raise RuntimeError(f"Missing rendered video frame: {name}")

    scene = bpy.context.scene
    sequence_editor = scene.sequence_editor_create()
    sequence = sequence_editor.strips.new_image(
        "Spatial Sketch export", os.path.join(frame_dir, frame_names[0]), 1, 1
    )
    for name in frame_names[1:]:
        sequence.elements.append(name)
    if len(sequence.elements) != frame_count:
        raise RuntimeError("Blender did not load the requested image sequence")

    scene.frame_start = 1
    scene.frame_end = frame_count
    scene.render.resolution_x = width
    scene.render.resolution_y = height
    scene.render.resolution_percentage = 100
    scene.render.fps = fps
    scene.render.image_settings.media_type = "VIDEO"
    scene.render.image_settings.file_format = "FFMPEG"
    scene.render.ffmpeg.format = "MPEG4"
    scene.render.ffmpeg.codec = "H264"
    scene.render.ffmpeg.audio_codec = "NONE"
    scene.render.ffmpeg.constant_rate_factor = "HIGH"
    scene.render.filepath = output_path
    scene.render.use_file_extension = True
    scene.render.use_sequencer = True
    scene.render.use_compositing = False
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.display_settings.display_device = "sRGB"

    bpy.ops.render.render(animation=True)

    if not os.path.isfile(output_path) or os.path.getsize(output_path) <= 64:
        raise RuntimeError("Blender did not produce a nonempty MP4")
    with open(output_path, "rb") as movie:
        movie.seek(4)
        if movie.read(4) != b"ftyp":
            raise RuntimeError("Blender output is not an MP4 container")
    print(f"SPATIAL_SKETCH_MP4_OK {frame_count} {width}x{height} {fps}fps")


if __name__ == "__main__":
    main()
